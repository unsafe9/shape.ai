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

import type { Scene, SceneComment, SceneSelection, Tag } from "../../shared/schema";
import type { RenderScenePatch } from "../../shared/renderPatch";
import type { WorldPoint } from "../../shared/renderScene";
import type { Command } from "../lib/commandCatalog";
import type { Object as SceneObject, ObjectOp, ObjectScene } from "../../shared/object";

/**
 * Result of `apply_object_op` (OB4.3). On success `scene` is the next object
 * scene and `inverse` is the captured inverse op (the undo entry, D21). On a
 * domain failure the scene is returned unchanged, `inverse` is null, and the
 * message rides `errors` — mirroring {@link RenderResult}'s error policy.
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
  [extra: string]: unknown;
};

/** A derived outline/region for an object's geometry (OB1.3, reference stub). */
export type DerivedRegion = Record<string, unknown>;

/** Result of `apply_render_patch` / `update_group_tags`. */
export type RenderResult = {
  scene: Scene;
  errors: string[];
};

/** Result of `add_comment`. `comment` is null when `errors` is non-empty. */
export type CommentResult = {
  scene: Scene;
  comment: SceneComment | null;
  errors: string[];
};

/** Insert ids for `insert_primitive_ops` (mirrors scene-core `InsertIds`). */
export type InsertIds = {
  primary: string;
  secondary: string;
  edge: string;
};

/** Primitive kind token accepted by `insert_primitive_ops`. */
export type PrimitiveSpec = "rectangle" | "ellipse" | "connector" | "sticky" | "frame";

// scene-core owns the template/recipe contract shapes; they are not yet mirrored
// as TS types, so they are surfaced as opaque records here. Tighten when a shared
// schema exists.
export type TemplateContract = Record<string, unknown>;
export type TemplateMetadata = Record<string, unknown>;
export type AppliedTemplate = Record<string, unknown>;

// Shape of the generated wasm-pack module (`shape_scene_core.js`). Declared
// locally — matching wasmLoader.ts — so this file does not statically import the
// gitignored build artifact's types; the dynamic import is `@vite-ignore`d.
//
// The module is built `--target web`, which exposes BOTH an async `default`
// (`__wbg_init`, browser: fetch the `.wasm` relative to the JS) and a sync
// `initSync` (init from already-loaded wasm bytes/module). The browser uses
// `default`; Node/vitest cannot `fetch()` the module, so it reads the `.wasm`
// from the filesystem and inits synchronously via `initSync`.
type SceneCoreModule = {
  default: (init?: unknown) => Promise<unknown>;
  initSync: (module: { module: BufferSource | WebAssembly.Module }) => unknown;
  apply_render_patch: (sceneJson: string, patchJson: string, now: string) => string;
  add_comment: (sceneJson: string, targetJson: string, body: string, now: string) => string;
  update_group_tags: (
    sceneJson: string,
    groupId: string,
    tagIdsJson: string,
    now: string
  ) => string;
  recipe_from_selection: (sceneJson: string, idsJson: string, metadataJson: string) => string;
  apply_template: (
    templateJson: string,
    anchorJson: string,
    idPrefix: string,
    now: string
  ) => string;
  insert_primitive_ops: (
    spec: string,
    anchorJson: string,
    groupId: string,
    idsJson: string,
    now: string
  ) => string;
  command_catalog: () => string;
  // OB4.3 object-native bridges (the same Rust the object-native server runs).
  apply_object_op: (sceneJson: string, opJson: string) => string;
  derive_region: (geometryJson: string, flatness: number) => string;
  object_command_catalog: () => string;
  build_object_template: (
    templateId: string,
    anchorX: number,
    anchorY: number,
    idPrefix: string
  ) => string;
};

/** Typed handle returned by {@link loadSceneCore}. */
export type SceneCore = {
  applyRenderPatch(scene: Scene, patch: RenderScenePatch, now: string): RenderResult;
  addComment(scene: Scene, target: SceneSelection, body: string, now: string): CommentResult;
  updateGroupTags(scene: Scene, groupId: string, tagIds: string[], now: string): RenderResult;
  recipeFromSelection(
    scene: Scene,
    ids: string[],
    metadata: TemplateMetadata
  ): TemplateContract;
  applyTemplate(
    template: TemplateContract,
    anchor: WorldPoint,
    idPrefix: string,
    now: string
  ): AppliedTemplate;
  insertPrimitiveOps(
    spec: PrimitiveSpec,
    anchor: WorldPoint,
    groupId: string,
    ids: InsertIds,
    now: string
  ): RenderScenePatch[];
  commandCatalog(): Command[];
  // OB4.3 object-native op-apply + derived contracts. These run the SAME Rust the
  // object-native server runs, so an object op applies identically on both sides.
  applyObjectOp(scene: ObjectScene, op: ObjectOp): ObjectApplyResult;
  deriveRegion(geometry: SceneObject["geometry"], flatness: number): DerivedRegion;
  objectCommandCatalog(): ObjectCommand[];
  buildObjectTemplate(
    templateId: string,
    anchorX: number,
    anchorY: number,
    idPrefix: string
  ): SceneObject[];
};

let modulePromise: Promise<SceneCoreModule> | null = null;
// Set once the wasm instance is initialized; backs the synchronous op-apply the
// sync engine uses on its hot path (after `ensureSceneCore` has resolved).
let readyModule: SceneCoreModule | null = null;

// `Tag` is part of scene-core's serde surface (tags ride inside Scene); imported
// to anchor the type contract even though no wrapper takes a bare Tag yet.
export type { Tag };

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
    applyRenderPatch(scene, patch, now) {
      return parseBridge<RenderResult>(
        "apply_render_patch",
        mod.apply_render_patch(JSON.stringify(scene), JSON.stringify(patch), now)
      );
    },
    addComment(scene, target, body, now) {
      return parseBridge<CommentResult>(
        "add_comment",
        mod.add_comment(JSON.stringify(scene), JSON.stringify(target), body, now)
      );
    },
    updateGroupTags(scene, groupId, tagIds, now) {
      return parseBridge<RenderResult>(
        "update_group_tags",
        mod.update_group_tags(JSON.stringify(scene), groupId, JSON.stringify(tagIds), now)
      );
    },
    recipeFromSelection(scene, ids, metadata) {
      return parseBridge<TemplateContract>(
        "recipe_from_selection",
        mod.recipe_from_selection(JSON.stringify(scene), JSON.stringify(ids), JSON.stringify(metadata))
      );
    },
    applyTemplate(template, anchor, idPrefix, now) {
      return parseBridge<AppliedTemplate>(
        "apply_template",
        mod.apply_template(JSON.stringify(template), JSON.stringify(anchor), idPrefix, now)
      );
    },
    insertPrimitiveOps(spec, anchor, groupId, ids, now) {
      return parseBridge<RenderScenePatch[]>(
        "insert_primitive_ops",
        mod.insert_primitive_ops(spec, JSON.stringify(anchor), groupId, JSON.stringify(ids), now)
      );
    },
    commandCatalog() {
      return parseBridge<Command[]>("command_catalog", mod.command_catalog());
    },
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
    }
  };
}

/**
 * Idempotently initialize the scene-core wasm instance so the synchronous
 * {@link applyRenderPatchSync} can run afterwards. The sync engine's owner awaits
 * this once (e.g. before `connect`) — the same `--target web` artifact backs both
 * the browser and Node/vitest, so the engine runs the SAME op-apply everywhere.
 */
export async function ensureSceneCore(): Promise<void> {
  await loadModule();
}

/**
 * Synchronous op-apply, the sync engine's hot path. Requires {@link ensureSceneCore}
 * to have resolved first (the wasm instance must be initialized); throws if not,
 * since the engine's `author`/`applyRemote`/`reconcileSnapshot` are synchronous
 * and cannot await an init. This is THE op-apply — the same Rust the server runs.
 */
export function applyRenderPatchSync(scene: Scene, patch: RenderScenePatch, now: string): RenderResult {
  if (!readyModule) {
    throw new Error("scene-core wasm is not initialized; await ensureSceneCore() before applyRenderPatchSync()");
  }
  return parseBridge<RenderResult>(
    "apply_render_patch",
    readyModule.apply_render_patch(JSON.stringify(scene), JSON.stringify(patch), now)
  );
}

/**
 * Synchronous object op-apply (OB4.3), the object sync engine's hot path. Like
 * {@link applyRenderPatchSync} it requires {@link ensureSceneCore} to have
 * resolved; it runs THE scene-core object op-apply — the same Rust the
 * object-native server runs — and returns the next scene plus the inverse op
 * (undo entry, D21).
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
