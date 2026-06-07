// OB4.3 / OB4.4 — object op-apply driven through the real scene-core wasm core.
//
// This is the Rust-first object path proof: vitest loads the SAME wasm the
// object-native server runs (`apply_object_op`) and asserts the object op union,
// inverse-op capture (D21), the derived-region contract (OB1.3), the object
// command catalog (OB3.S9), and template lowering (build_object_template). There
// is NO TS op-apply here — all domain logic lives in the wasm core (P1).

import { beforeAll, describe, expect, it } from "vitest";

import {
  ensureSceneCore,
  loadSceneCore,
  applyObjectOpSync,
  type SceneCore
} from "../src/client/scene/sceneCoreWasm";
import {
  emptyObjectScene,
  translateTransform,
  IDENTITY_TRANSFORM,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene
} from "../src/shared/object";

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

/** A closed unit rect at object-local (0,0)-(80,40), quantized integer coords. */
function rect(id: string, order = "a0"): SceneObject {
  return {
    id,
    order,
    transform: IDENTITY_TRANSFORM,
    geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "evenOdd" }
  };
}

function insert(object: SceneObject): ObjectOp {
  return { kind: "insert-object", object };
}

describe("object op-apply (scene-core wasm)", () => {
  it("inserts an object and captures the inverse delete", () => {
    const scene = emptyObjectScene();
    const result = core.applyObjectOp(scene, insert(rect("rect-1")));
    expect(result.errors).toEqual([]);
    expect(result.scene.objects.map((o) => o.id)).toEqual(["rect-1"]);
    // Inverse of an insert is a delete of the same id (D21).
    expect(result.inverse).toEqual({ kind: "delete", id: "rect-1" });
  });

  it("moves via set-transform with zero geometry rebake (P4) and an inverse", () => {
    const inserted = core.applyObjectOp(emptyObjectScene(), insert(rect("rect-1")));
    const geomBefore = inserted.scene.objects[0].geometry;

    const move: ObjectOp = {
      kind: "set-transform",
      id: "rect-1",
      transform: translateTransform(100, 50)
    };
    const result = core.applyObjectOp(inserted.scene, move);
    expect(result.errors).toEqual([]);
    const moved = result.scene.objects[0];
    // Zero-rebake: a transform edit must not touch the geometry.
    expect(moved.geometry).toEqual(geomBefore);
    expect(moved.transform).toEqual(translateTransform(100, 50));
    // The wire transform is a bare 3x3 array, not `{ m: [...] }`.
    expect(Array.isArray(moved.transform)).toBe(true);
    // The inverse restores the identity transform.
    expect(result.inverse).toEqual({
      kind: "set-transform",
      id: "rect-1",
      transform: IDENTITY_TRANSFORM
    });
  });

  it("undoes by authoring the inverse op through the same path (D21)", () => {
    const base = core.applyObjectOp(emptyObjectScene(), insert(rect("r"))).scene;
    const move: ObjectOp = { kind: "set-transform", id: "r", transform: translateTransform(5, 5) };
    const after = core.applyObjectOp(base, move);
    const undone = core.applyObjectOp(after.scene, after.inverse!);
    // Authoring the inverse restores the prior transform.
    expect(undone.scene.objects[0].transform).toEqual(base.objects[0].transform);
  });

  it("rejects a domain-invalid op without mutating the scene", () => {
    const scene = emptyObjectScene();
    // set-transform on an absent object is a domain failure: scene unchanged,
    // inverse null, message in errors (NOT thrown across the FFI boundary).
    const result = core.applyObjectOp(scene, {
      kind: "set-transform",
      id: "ghost",
      transform: translateTransform(1, 1)
    });
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.inverse).toBeNull();
    expect(result.scene.objects).toEqual([]);
  });

  it("applies a batch atomically as one undo unit", () => {
    const batch: ObjectOp = {
      kind: "batch",
      ops: [insert(rect("a", "a0")), insert(rect("b", "a1"))]
    };
    const result = core.applyObjectOp(emptyObjectScene(), batch);
    expect(result.errors).toEqual([]);
    expect(result.scene.objects.map((o) => o.id).sort()).toEqual(["a", "b"]);
    expect(result.inverse).not.toBeNull();
  });

  it("runs the synchronous applyObjectOpSync hot path", () => {
    const result = applyObjectOpSync(emptyObjectScene(), insert(rect("sync-1")));
    expect(result.errors).toEqual([]);
    expect(result.scene.objects[0].id).toBe("sync-1");
  });
});

describe("derived region contract (OB1.3)", () => {
  it("derives a closed-rect region from geometry", () => {
    const region = core.deriveRegion(
      { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "evenOdd" },
      1
    ) as { closed?: boolean; bounds?: { minX: number; minY: number; maxX: number; maxY: number } };
    expect(region.closed).toBe(true);
    expect(region.bounds).toEqual({ minX: 0, minY: 0, maxX: 80, maxY: 40 });
  });
});

describe("object command catalog (OB3.S9)", () => {
  it("returns a non-empty catalog with id/label/category rows", () => {
    const catalog = core.objectCommandCatalog();
    expect(catalog.length).toBeGreaterThan(0);
    for (const command of catalog) {
      expect(typeof command.id).toBe("string");
      expect(typeof command.label).toBe("string");
      expect(typeof command.category).toBe("string");
    }
  });
});

describe("template lowering (build_object_template)", () => {
  it("builds a recipe of inline-styled objects and round-trips through op-apply", () => {
    // `todo_board` is one of the crate's template ids (templates.rs registry).
    const templateId = "todo_board";
    const recipe = core.buildObjectTemplate(templateId, 100, 200, "tpl");
    expect(Array.isArray(recipe)).toBe(true);
    expect(recipe.length).toBeGreaterThan(0);

    // The lowered recipe applies as a batch of insert-object ops (the shell sends
    // these as a FeatureRequest.templateApply; the server lowers them server-side,
    // but they are valid object-op inserts here too).
    let scene: ObjectScene = emptyObjectScene();
    for (const object of recipe) {
      const result = applyObjectOpSync(scene, insert(object));
      expect(result.errors).toEqual([]);
      scene = result.scene;
    }
    expect(scene.objects.length).toBe(recipe.length);
  });
});
