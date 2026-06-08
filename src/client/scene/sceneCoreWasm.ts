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

import type { Object as SceneObject, ObjectOp, ObjectScene } from "../../shared/object";

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

/** A derived outline/region for an object's geometry (OB1.3, reference stub). */
export type DerivedRegion = Record<string, unknown>;

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
    epsilon: number,
    id: string,
    order: string
  ) => string;
  WasmUndoStack: new (actorId: string) => WasmUndoStack;
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
  buildObjectTemplate(
    templateId: string,
    anchorX: number,
    anchorY: number,
    idPrefix: string
  ): SceneObject[];
  /** FC-11: lower one freehand stroke (world-px points) to a committed `Object`
   *  with an object-local geometry + brush stroke. Points become a JSON array of
   *  `[x, y]` pairs for the wasm bridge. */
  freehandToObject(
    points: { x: number; y: number }[],
    color: string,
    widthPx: number,
    epsilon: number,
    id: string,
    order: string
  ): SceneObject;
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
    buildObjectTemplate(templateId, anchorX, anchorY, idPrefix) {
      return parseBridge<SceneObject[]>(
        "build_object_template",
        mod.build_object_template(templateId, anchorX, anchorY, idPrefix)
      );
    },
    freehandToObject(points, color, widthPx, epsilon, id, order) {
      return parseBridge<SceneObject>(
        "freehand_to_object",
        mod.freehand_to_object(
          JSON.stringify(points.map((p) => [p.x, p.y])),
          color,
          widthPx,
          epsilon,
          id,
          order
        )
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
