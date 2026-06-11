// MG0.4: typed lazy loader for the scene-core wasm bridge.
//
// scene-core (crates/scene-core) is the single source of truth for op-apply. The
// server links it as a Rust crate; the web client loads the SAME logic compiled
// to wasm via this loader, so a patch applies identically on both sides.
//
// Each wrapper mirrors one `#[wasm_bindgen]` export in
// `crates/scene-core/src/wasm_api.rs`: it passes JSON strings in, and
// `JSON.parse`s the JSON string the bridge returns. The bridge never throws
// across the FFI boundary — a malformed-input or serialize failure comes back as
// `{ error: string }`, which these wrappers surface by throwing a typed Error so
// callers can use plain try/catch. Domain failures (unknown id, invalid patch)
// are NOT thrown: they ride in the `errors: string[]` field of the result.

import type { Anchor, Object as SceneObject, ObjectOp, ObjectScene, Transform3x3 } from "../shared/object";
import type { DragSpan } from "../controller/objectPrimitives";
import type { PrimitiveKindId } from "../controller/toolbar";

/**
 * Tier-2 `move_ops` roots: a single dragged object, or a multi-select set. The
 * wire shape the scene-core bridge decodes (`{kind:"single",id} |
 * {kind:"multi",ids}`).
 */
export type MoveRoots = { kind: "single"; id: string } | { kind: "multi"; ids: string[] };

/**
 * Result of `apply_object_op` (OB4.3). On success `scene` is the next object
 * scene and `inverse` is the captured inverse op (the undo entry, D21). On a
 * domain failure the scene is returned unchanged, `inverse` is null, and the
 * message rides `errors`.
 */
export type ObjectApplyResult = {
  scene: ObjectScene;
  inverse: ObjectOp | null;
  errors: string[];
};

/** A row of the object command catalog (label/category/shortcut/op mapping). */
export type ObjectCommand = {
  id: string;
  label: string;
  category: string;
  defaultShortcut?: string;
  description?: string;
  /** The ObjectOp kind this command lowers to 1:1, when applicable. */
  opKind?: string;
  [extra: string]: unknown;
};

/**
 * The hold-key trigger for a gesture (C2). Exactly one of `key`/`button`/
 * `modifier` is populated, the one matching `input`; `degrees` carries an
 * optional numeric parameter (e.g. the coarse-rotate step).
 */
export type HoldTrigger = {
  input: "key" | "button" | "modifier";
  key?: string;
  button?: string;
  modifier?: string;
  degrees?: number;
};

/** A row of the object gesture catalog (press-and-hold input gestures, C2). */
export type ObjectGesture = {
  id: string;
  label: string;
  category: string;
  trigger: HoldTrigger;
  description: string;
};

/** A derived outline/region for an object's geometry (OB1.3, reference stub). */
export type DerivedRegion = Record<string, unknown>;

/**
 * Tier-4 container-vs-leaf decision for a double-click (mirrors the core
 * `DoubleClickAction`): a container drills in, a leaf edits its text. The shell
 * dispatches the action (set active-container vs inline text edit).
 */
export type DoubleClickAction = { kind: "drill-in-container" } | { kind: "edit-leaf" };

/**
 * Pen recognition mode (core `RecognizeMode`): `"basic"` force-snaps every
 * stroke to a basic primitive (line / ellipse / rect / triangle), `"free"`
 * runs the full pipeline with polygon + silhouette fallbacks.
 */
export type RecognizeMode = "basic" | "free";

// Shape of the generated wasm-pack module (`shape_scene_core.js`). Declared
// locally — matching wasmLoader.ts — so this file does not statically import the
// gitignored build artifact's types; the dynamic import is `@vite-ignore`d.
//
// The module is built `--target web`, which exposes BOTH an async `default`
// (`__wbg_init`, browser: fetch the `.wasm` relative to the JS) and a sync
// `initSync` (init from already-loaded wasm bytes/module). The browser uses
// `default`; Node/vitest cannot `fetch()` the module, so it reads the `.wasm`
// from the filesystem and inits synchronously via `initSync`.
//
// Only the OB4.3 object-native bridges are surfaced here; the legacy
// Group/Card/Edge wasm exports remain in the binary but are no longer bound on
// the TS side (a follow-up removes them from the crate entirely).
type SceneCoreModule = {
  default: (init?: unknown) => Promise<unknown>;
  initSync: (module: { module: BufferSource | WebAssembly.Module }) => unknown;
  apply_object_op: (sceneJson: string, opJson: string) => string;
  derive_region: (geometryJson: string, flatness: number) => string;
  object_command_catalog: () => string;
  object_gesture_catalog: () => string;
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
  partial_erase_ops: (sceneJson: string, id: string, x: number, y: number, radius: number) => string;
  anchor_follow_ops: (sceneJson: string, transformOpsJson: string) => string;
  move_ops: (sceneJson: string, rootsJson: string, deltaJson: string) => string;
  synthesize_create_anchors: (
    createdJson: string,
    targetJson: string,
    endpointX: number,
    endpointY: number
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

// The collaboration session (crates/client-runtime), exported as a wasm-bindgen
// class. It owns the optimistic sync engine + the peer registry; all payloads
// cross the boundary as JSON strings. Method names stay snake_case. The TS runtime
// adapters (syncEngine.ts / peers.ts / outbox.ts bookkeeping) drive this; the IO
// seams (WS socket, IndexedDB durability, timers) stay TS-side.
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
  ingest_presence: (payloadJson: string, nowMs: number) => string;
  expire_peers: (nowMs: number) => string;
  peers: () => string;
  clear_peers: () => void;
  free: () => void;
};

// The viewport-windowing DECISION layer (crates/client-runtime WindowState),
// exported as a wasm-bindgen class. It owns the subscribed window + margin and
// decides when a re-subscribe is warranted; the shell drives the debounce timer
// and the transport. Bboxes cross as `{x,y,width,height}` JSON; `""`/`"null"` is
// whole-canvas. Method names stay snake_case.
type WasmWindow = {
  on_viewport: (viewportJson: string) => string;
  set_window: (bboxJson: string) => string;
  subscribe_whole_canvas: () => boolean;
  current_window: () => string;
  free: () => void;
};

// The core's per-actor undo stack (FC-15), exported as a wasm-bindgen class. Ops
// cross the boundary as ObjectOp JSON; `undo`/`redo` return the op JSON to apply
// (or `undefined` when the stack is empty). Method names stay snake_case.
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

/**
 * Per-actor undo/redo stack (FC-15), backed by the core's `UndoStack` (D21). It
 * owns no scene and performs no apply: `undo`/`redo` hand back the op the caller
 * must re-author through the SAME `applyObjectOp` path, then the caller reports
 * the re-inverse via `noteUndoApplied`/`noteRedoApplied`. A continuous gesture
 * collapses to one step inside a `beginCoalesce`/`endCoalesce` window.
 */
export type UndoStack = {
  /** Record an applied edit (forward op + the inverse op-apply returned). A
   *  fresh record clears redo; during a coalesce window it folds into one entry. */
  record(forward: ObjectOp, inverse: ObjectOp): void;
  beginCoalesce(): void;
  endCoalesce(): void;
  isCoalescing(): boolean;
  /** The inverse op to apply, or null when nothing to undo. */
  undo(): ObjectOp | null;
  /** Report the re-inverse `applyObjectOp` returned for the undo op. */
  noteUndoApplied(reInverse: ObjectOp): void;
  /** The forward op to re-apply, or null when nothing to redo. */
  redo(): ObjectOp | null;
  /** Report the inverse `applyObjectOp` returned for the redo op. */
  noteRedoApplied(inverse: ObjectOp): void;
  /** Abort an in-flight undo/redo handshake whose apply failed (clears pending). */
  abort(): void;
  canUndo(): boolean;
  canRedo(): boolean;
};

/** Typed handle returned by {@link loadSceneCore}. The object-native op-apply +
 *  derived contracts run the SAME Rust the object-native server runs. */
export type SceneCore = {
  applyObjectOp(scene: ObjectScene, op: ObjectOp): ObjectApplyResult;
  deriveRegion(geometry: SceneObject["geometry"], flatness: number): DerivedRegion;
  objectCommandCatalog(): ObjectCommand[];
  objectGestureCatalog(): ObjectGesture[];
  buildObjectTemplate(
    templateId: string,
    anchorX: number,
    anchorY: number,
    idPrefix: string
  ): SceneObject[];
  /** FC-11 / anchor-semantics v3 §4: RECOGNIZE one freehand stroke (world-px
   *  points) at pen-up and commit it as one `Object`, per `mode` — `"basic"`
   *  force-snaps to a basic primitive (line / ellipse / rect / triangle),
   *  `"free"` runs the full pipeline (line / ellipse / rect / polygon /
   *  normalized silhouette, open or closed) — with an object-local geometry +
   *  brush stroke. Points become a JSON array of `[x, y]` pairs for the wasm
   *  bridge. */
  freehandToObject(
    points: { x: number; y: number }[],
    color: string,
    widthPx: number,
    id: string,
    order: string,
    mode: RecognizeMode
  ): SceneObject;
  /** Tier-3: build a basic primitive (rectangle/ellipse/line/text/frame) centered
   *  on a world anchor, in `color` (omit for the kind default — may be a hex or the
   *  theme-default sentinel). The geometry is object-local; the world position rides
   *  a translate (P4). The shell sends the result as an `insert-object` op. */
  buildPrimitive(
    kind: PrimitiveKindId,
    anchor: { x: number; y: number },
    id: string,
    order: string,
    color?: string
  ): SceneObject;
  /** Tier-3: build a primitive sized to a drag `span` — closed kinds to the
   *  normalized bbox, the line corner-to-corner. Same color rules as
   *  {@link buildPrimitive}. */
  buildPrimitiveFromDrag(
    kind: PrimitiveKindId,
    span: DragSpan,
    id: string,
    order: string,
    color?: string
  ): SceneObject;
  /** Tier-3: author a `set-style` op recoloring `object` to `color` (hex or the
   *  theme-default sentinel). Recolors only existing style fields; a borderless
   *  object gains a fill. The shell authors the op through the same op-apply path. */
  buildSetStyleOp(object: SceneObject, color: string): ObjectOp;
  /** Anchor-semantics v3 §1: the open/closed data-level dichotomy for a path
   *  string — true iff the geometry is exactly one OPEN subpath. Class-dependent
   *  shell branches (Alt-detach, fill-vs-stroke routing) consult THE core
   *  classifier instead of re-parsing geometry in TS. */
  isOpenClassD(d: string): boolean;
  /** v3 §4 multi-stroke merge: the ops merging a released freehand stroke
   *  (world-px points) into the open-class object(s) whose endpoint(s) its
   *  ends landed within `tolerancePx` (WORLD px — divide the screen-px
   *  constant by the zoom) — one `edit-geometry` on the survivor plus the
   *  anchor release / absorbed-object delete, batch-ready, never an insert.
   *  Null = no merge: the shell keeps its existing insert + release-anchoring
   *  path. Merge takes priority over release-anchor authoring. */
  mergeOpenStrokeOps(
    scene: ObjectScene,
    points: { x: number; y: number }[],
    mode: RecognizeMode,
    tolerancePx: number
  ): ObjectOp[] | null;
  /** W2-08: partial erase — cut a stroke's geometry at an object-local quantized
   *  touch point + radius. Returns the new geometry (two open subpaths around the
   *  removed node), or null when the touch missed every node (nothing to cut). */
  splitSubpathAt(
    geometry: SceneObject["geometry"],
    x: number,
    y: number,
    radius: number
  ): SceneObject["geometry"] | null;
  /** #2/#3: cut the stroke at an object-local quantized touch and return the WHOLE
   *  op batch — `[]` on a miss, `[delete]` when the cut empties the object, else
   *  `[edit-geometry, ...follower-reprojection]` (a reshape re-projects anchored
   *  followers onto the new outline, chained so a follower of a follower follows
   *  too). The shell authors the result and owns only the UI follow-up; op
   *  orchestration stays in the core. (`moveOps` / `endpointReleaseOps` fold the
   *  same follow into their own batches.) */
  partialEraseOps(scene: ObjectScene, id: string, x: number, y: number, radius: number): ObjectOp[];
  /** #2/#3: the commit-time anchor follow ops for an arbitrary committed batch —
   *  `set-transform` MOVES a target, `edit-geometry` RESHAPES one, and the result is
   *  the chord-deform ops that make every anchored follower track its target
   *  (chained). The op-authoring bridges (`partialEraseOps`, `endpointReleaseOps`,
   *  `moveOps`) already fold this into their own batches; this is the standalone
   *  entry. Returns `[]` when nothing follows. */
  anchorFollowOps(scene: ObjectScene, ops: ObjectOp[]): ObjectOp[];
  /** Tier-2: the combined commit-time move ops for a parent-drag / multi-select
   *  drag — the transform CASCADE ops (the dragged subtree, or every multi member
   *  and its subtree, deduped) FOLLOWED BY the anchor-follow `edit-geometry` ops
   *  those moves trigger, as ONE batch-ready array (cascade BEFORE follow). `delta`
   *  is the world-space gesture matrix. Collapses the shell commit to one call. */
  moveOps(scene: ObjectScene, roots: MoveRoots, delta: Transform3x3): ObjectOp[];
  /** AP5: synthesize the persistent anchor(s) for a snapped drag-create binding
   *  `created`'s endpoint node to `target`, or null when no anchor should be
   *  authored (target is the created object, or no node / no snap). */
  synthesizeCreateAnchors(
    created: SceneObject,
    target: SceneObject,
    endpoint: { x: number; y: number }
  ): Anchor[] | null;
  /** Anchor-semantics v3 §2b: the commit ops of an endpoint-drag release on an
   *  open-class object — one chord-deform `edit-geometry` moving the endpoint
   *  (`nodeIndex`, 0 or last) to the world-px release point, plus the
   *  `set-anchor` whole-vector rewrite: rebound to `snap.targetId` (at the
   *  snapped world point `snap.at`, defaulting to the release point) when the
   *  release snapped, or that endpoint's anchor removed when `snap` is null.
   *  Returns `[]` when the op does not apply (unknown id, closed-class,
   *  interior node). The shell batches the returned ops. */
  endpointReleaseOps(
    scene: ObjectScene,
    id: string,
    nodeIndex: number,
    newPoint: { x: number; y: number },
    snap: { targetId: string; at?: { x: number; y: number } } | null
  ): ObjectOp[];
  /** Tier-4 (#18): the `reparent` op that pops `id` out one level (to its
   *  grandparent, or to the canvas root when the parent sits at the root),
   *  preserving its order key. Null when `id` is unknown or already at the root.
   *  The shell authors the returned op through the same op-apply path. */
  popOutOp(scene: ObjectScene, id: string): ObjectOp | null;
  /** Tier-4 (#13): whether `id` is a container (has at least one child). */
  hasChildren(scene: ObjectScene, id: string): boolean;
  /** Tier-4 (#13): whether ungroup is enabled for the single selected object —
   *  true only when a non-null `selectedId` is a container (has children). */
  ungroupEnabled(scene: ObjectScene, selectedId: string | null): boolean;
  /** Tier-4 (#9): the container-vs-leaf decision for a double-click on `id`. The
   *  shell dispatches the action (drill-in vs inline text edit). */
  doubleClickAction(scene: ObjectScene, id: string): DoubleClickAction;
  /** FC-15: create a per-actor undo/redo stack backed by the core (D21). */
  createUndoStack(actorId: string): UndoStack;
};

let modulePromise: Promise<SceneCoreModule> | null = null;
// Set once the wasm instance is initialized; backs the synchronous op-apply the
// sync engine uses on its hot path (after `ensureSceneCore` has resolved).
let readyModule: SceneCoreModule | null = null;

/** True under Node/vitest (no `fetch`-served wasm), false in the browser. */
function isNodeRuntime(): boolean {
  return (
    typeof process !== "undefined" &&
    process.versions != null &&
    process.versions.node != null &&
    typeof (globalThis as { window?: unknown }).window === "undefined"
  );
}

/**
 * Init the wasm instance under Node/vitest. The `--target web` module cannot
 * `fetch()`, so read the sibling `.wasm` from disk and init synchronously via
 * `initSync`. The path is resolved from this file's URL so it works regardless of
 * the test's cwd. Built by `npm run scene:wasm:build` into `./wasm`.
 *
 * The `node:` specifiers are assembled at runtime and `@vite-ignore`d so Vite's
 * browser bundler never statically resolves (or warns about) them — this branch
 * is only ever reached under Node, never in the browser.
 */
async function initUnderNode(mod: SceneCoreModule): Promise<void> {
  const nodeImport = (name: string) => import(/* @vite-ignore */ `node:${name}`);
  const { readFileSync } = (await nodeImport("fs")) as typeof import("node:fs");
  const { fileURLToPath } = (await nodeImport("url")) as typeof import("node:url");
  const wasmUrl = new URL("./wasm/shape_scene_core_bg.wasm", import.meta.url);
  const bytes = readFileSync(fileURLToPath(wasmUrl));
  mod.initSync({ module: bytes });
}

/**
 * Parse a bridge return string. If it is the `{ error }` shape the bridge emits
 * for a malformed input or serialize failure, throw; otherwise return the parsed
 * value. Domain `errors[]` fields pass through untouched.
 */
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
      // Browser: `default` fetches the sibling `.wasm`. Node/vitest: read the
      // bytes from disk and init synchronously (no `fetch`).
      if (isNodeRuntime()) await initUnderNode(mod);
      else await mod.default();
      readyModule = mod;
      return mod;
    })();
  }
  return modulePromise;
}

/**
 * Lazy-load the scene-core wasm package and return a typed handle. Idempotent:
 * the underlying module + wasm instance are initialized once and shared.
 */
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
      // A missed touch comes back as `{error}` (nothing to cut); treat that as a
      // no-op (null) rather than a thrown failure — only a malformed input throws.
      const raw = mod.split_subpath_at(JSON.stringify(geometry), x, y, radius);
      const value = JSON.parse(raw) as SceneObject["geometry"] | { error: string };
      if (value && typeof value === "object" && "error" in value && typeof value.error === "string") {
        return null;
      }
      return value as SceneObject["geometry"];
    },
    partialEraseOps(scene, id, x, y, radius) {
      return parseBridge<ObjectOp[]>(
        "partial_erase_ops",
        mod.partial_erase_ops(JSON.stringify(scene), id, x, y, radius)
      );
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
    createUndoStack(actorId) {
      const inner = new mod.WasmUndoStack(actorId);
      // `undo`/`redo` return the op JSON to apply (or `undefined` when empty).
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

/**
 * Idempotently initialize the scene-core wasm instance so the synchronous
 * {@link applyObjectOpSync} can run afterwards. The sync engine's owner awaits
 * this once (e.g. before `connect`) — the same `--target web` artifact backs both
 * the browser and Node/vitest, so the engine runs the SAME op-apply everywhere.
 */
export async function ensureSceneCore(): Promise<void> {
  await loadModule();
}

/**
 * Synchronous object op-apply (OB4.3), the object sync engine's hot path.
 * Requires {@link ensureSceneCore} to have resolved first (the wasm instance must
 * be initialized); throws if not, since the engine's `author`/`applyRemote`/
 * `reconcileSnapshot` are synchronous and cannot await an init. It runs THE
 * scene-core object op-apply — the same Rust the object-native server runs — and
 * returns the next scene plus the inverse op (undo entry, D21).
 */
export function applyObjectOpSync(scene: ObjectScene, op: ObjectOp): ObjectApplyResult {
  if (!readyModule) {
    throw new Error("scene-core wasm is not initialized; await ensureSceneCore() before applyObjectOpSync()");
  }
  return parseBridge<ObjectApplyResult>(
    "apply_object_op",
    readyModule.apply_object_op(JSON.stringify(scene), JSON.stringify(op))
  );
}

/** The raw collaboration session FFI handle (snake_case, JSON over the boundary).
 *  The TS runtime adapters wrap it with their stable, JSON-marshalling API. */
export type { WasmSession, WasmWindow };

/**
 * Construct a collaboration session over the wasm core (the SAME bundle as the
 * op-apply). Requires {@link ensureSceneCore} to have resolved (the wasm instance
 * must be initialized), since the session methods are synchronous. `coalesceMs <
 * 0` / `peerTtlMs < 0` use the core defaults; an empty `selfUserId` disables peer
 * self-skip. Throws if the wasm is not yet initialized.
 */
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

/**
 * Construct the viewport-windowing decision state over the wasm core (the SAME
 * bundle as the op-apply). Requires {@link ensureSceneCore} to have resolved. The
 * `seed` bbox is the connect region's window (omit for whole-canvas); `margin <
 * 0` uses the core default. Throws if the wasm is not yet initialized.
 */
export function createWasmWindow(args: {
  seed?: { x: number; y: number; width: number; height: number };
  margin?: number;
}): WasmWindow {
  if (!readyModule) {
    throw new Error("scene-core wasm is not initialized; await ensureSceneCore() before createWasmWindow()");
  }
  return new readyModule.WasmWindow(args.seed ? JSON.stringify(args.seed) : "", args.margin ?? -1);
}
