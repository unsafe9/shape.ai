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
type SceneCoreModule = {
  default: (init?: unknown) => Promise<unknown>;
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
};

let modulePromise: Promise<SceneCoreModule> | null = null;

// `Tag` is part of scene-core's serde surface (tags ride inside Scene); imported
// to anchor the type contract even though no wrapper takes a bare Tag yet.
export type { Tag };

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
      await mod.default();
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
    }
  };
}
