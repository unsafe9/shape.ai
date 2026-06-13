import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { emptyObjectScene, type Object as SceneObject, type ObjectScene, type ObjectSelection } from "../shared/object";
import {
  CANVAS_MENU,
  OBJECT_MENU,
  popOutPickEnabled,
  resolveContextMenuItems,
  resolveDoubleClick,
  ungroupPickEnabled
} from "../controller/interactions";

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

function obj(id: string, parent: string | undefined): SceneObject {
  return {
    id,
    ...(parent ? { parent } : {}),
    order: "a0",
    geometry: { d: "M 0 0 L 8 0" }
  } as SceneObject;
}

function sceneOf(objects: SceneObject[]): ObjectScene {
  return { ...emptyObjectScene(), objects };
}

describe("doubleClickAction (drill-in branch) — core query", () => {
  const scene = sceneOf([obj("frame", undefined), obj("child", "frame"), obj("leaf", undefined)]);

  it("drills into a container (has children) and edits a leaf", () => {
    expect(core.doubleClickAction(scene, "frame")).toEqual({ kind: "drill-in-container" });
    expect(core.doubleClickAction(scene, "leaf")).toEqual({ kind: "edit-leaf" });
  });
});

describe("ungroupEnabled — core query", () => {
  const scene = sceneOf([obj("frame", undefined), obj("child", "frame"), obj("leaf", undefined)]);

  it("is enabled for a container object (one WITH children)", () => {
    expect(core.hasChildren(scene, "frame")).toBe(true);
    expect(core.ungroupEnabled(scene, "frame")).toBe(true);
  });

  it("is disabled for a childless object", () => {
    expect(core.hasChildren(scene, "leaf")).toBe(false);
    expect(core.ungroupEnabled(scene, "leaf")).toBe(false);
  });

  it("is disabled when nothing is the single picked object (null id)", () => {
    expect(core.ungroupEnabled(scene, null)).toBe(false);
  });
});

describe("popOutOp (pop a child out one level) — core query", () => {
  // root -> mid -> deep.
  const scene = sceneOf([obj("root", undefined), obj("mid", "root"), obj("deep", "mid"), obj("top", undefined)]);

  it("reparents a child to its GRANDPARENT", () => {
    expect(core.popOutOp(scene, "deep")).toEqual({ kind: "reparent", id: "deep", parent: "root", order: "a0" });
  });

  it("reparents to the canvas ROOT (absent parent) when the parent sits at the root", () => {
    expect(core.popOutOp(scene, "mid")).toEqual({ kind: "reparent", id: "mid", order: "a0" });
  });

  it("is a no-op for a root-level object (nothing to pop out of) or an unknown id", () => {
    expect(core.popOutOp(scene, "top")).toBeNull();
    expect(core.popOutOp(scene, "ghost")).toBeNull();
  });
});

describe("controller group/hierarchy wiring", () => {
  function geoObj(id: string, tx: number, ty: number): SceneObject {
    return {
      id,
      order: "a0",
      transform: [
        [1, 0, tx],
        [0, 1, ty],
        [0, 0, 1]
      ],
      geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z" }
    } as SceneObject;
  }

  it("groupOps groups 2+ objects — insert a fresh frame, then reparent each child under it", () => {
    const scene = sceneOf([geoObj("a", 100, 100), geoObj("b", 220, 100)]);
    const ops = core.groupOps(scene, ["a", "b"], "frame-1")!;
    expect(ops[0]).toMatchObject({ kind: "insert-object", object: { id: "frame-1" } });
    expect(ops.slice(1)).toEqual([
      { kind: "reparent", id: "a", parent: "frame-1", order: "a0" },
      { kind: "reparent", id: "b", parent: "frame-1", order: "a1" }
    ]);
  });

  it("groupOps requires 2+ known members — a single member yields null (the core-owned contract)", () => {
    const scene = sceneOf([geoObj("a", 100, 100)]);
    expect(core.groupOps(scene, ["a"], "frame-1")).toBeNull();
  });

  it("duplicateOps clones at the canonical +40/+40 offset with core-minted ids + fractional order keys", () => {
    // The App duplicateSelection routes ids through duplicateOps(scene, ids, idPrefix, 0) — the core owns
    // the offset, the fresh `{idPrefix}-{n}` ids, and the order keys (no shell `~` minting).
    const scene = sceneOf([geoObj("a", 100, 100), geoObj("b", 200, 50)]);
    const ops = core.duplicateOps(scene, ["a", "b"], "dup-xyz", 0);
    expect(ops.map((o) => (o.kind === "insert-object" ? o.object.id : ""))).toEqual(["dup-xyz-0", "dup-xyz-1"]);
    const first = ops[0];
    if (first.kind !== "insert-object") throw new Error("expected insert-object");
    // +40/+40 down-right of the source's (100, 100) translate.
    expect([first.object.transform![0][2], first.object.transform![1][2]]).toEqual([140, 140]);
    // Order keys are real fractional keys (no `~` suffix the shell used to mint).
    expect(ops.every((o) => o.kind === "insert-object" && !o.object.order.includes("~"))).toBe(true);
  });

  it("reorderStepOps swaps a single object's order with its flat-order neighbor (2-op swap, or null at the extent)", () => {
    // The App reorderStep routes through reorderStepOps(scene, id, dir) — the core authors the swap.
    const scene = sceneOf([
      { ...geoObj("a", 0, 0), order: "a0" } as SceneObject,
      { ...geoObj("b", 0, 0), order: "a1" } as SceneObject
    ]);
    expect(core.reorderStepOps(scene, "a", "forward")).toEqual([
      { kind: "reorder", id: "a", order: "a1" },
      { kind: "reorder", id: "b", order: "a0" }
    ]);
    // The top object has no forward neighbor — null, not a malformed swap.
    expect(core.reorderStepOps(scene, "b", "forward")).toBeNull();
  });

  it("resolveDoubleClick branches through the core drill-in decision (container vs leaf)", () => {
    const scene = sceneOf([obj("frame", undefined), obj("child", "frame"), obj("leaf", undefined)]);
    expect(resolveDoubleClick(core, scene, { id: "frame", hasChildren: true })).toEqual({ kind: "drill-in", id: "frame" });
    expect(resolveDoubleClick(core, scene, { id: "leaf", hasChildren: false })).toEqual({ kind: "edit-leaf", id: "leaf" });
    expect(resolveDoubleClick(core, scene, null)).toEqual({ kind: "none" });
  });

  it("gates ungroup + pop-out on the core children/parent queries", () => {
    const scene = sceneOf([obj("frame", undefined), obj("child", "frame"), obj("leaf", undefined), obj("deep", "child")]);
    expect(ungroupPickEnabled(core, scene, { kind: "object", id: "frame" })).toBe(true);
    expect(ungroupPickEnabled(core, scene, { kind: "object", id: "leaf" })).toBe(false);
    expect(ungroupPickEnabled(core, scene, { kind: "multi", ids: ["frame"] })).toBe(false);
    expect(popOutPickEnabled(core, scene, { kind: "object", id: "child" })).toBe(true);
    expect(popOutPickEnabled(core, scene, { kind: "object", id: "frame" })).toBe(false);
  });

  it("the OBJECT_MENU carries the pop-out entry; the CANVAS_MENU drops insert-text", () => {
    expect(OBJECT_MENU).toContainEqual({ id: "pop-out", label: "Pop out one level" });
    const canvasIds = CANVAS_MENU.filter((e): e is Exclude<typeof e, "separator"> => e !== "separator").map((e) => e.id);
    expect(canvasIds).not.toContain("insert-text");
    expect(canvasIds).toContain("insert-rectangle");
  });

  it("resolveContextMenuItems gates ungroup/pop-out on the picked target and drops handler-less entries", () => {
    const scene = sceneOf([obj("frame", undefined), obj("child", "frame"), obj("leaf", undefined)]);
    const picked: ObjectSelection = { kind: "object", id: "leaf" };
    const enabledFor = (entry: { id: string }, p: ObjectSelection) => {
      if (entry.id === "ungroup") return ungroupPickEnabled(core, scene, p);
      if (entry.id === "pop-out") return popOutPickEnabled(core, scene, p);
      return true;
    };
    // Only entries with a handler survive; a leaf disables ungroup + pop-out.
    const items = resolveContextMenuItems<string>(picked, [], (id) => id !== "add-comment", enabledFor);
    const present = items.filter((i): i is NonNullable<typeof i> => i !== null);
    const byId = new Map(present.map((i) => [i.id, i]));
    expect(byId.has("add-comment")).toBe(false);
    expect(byId.get("ungroup")?.disabled).toBe(true);
    expect(byId.get("pop-out")?.disabled).toBe(true);
    // group is disabled for a single object.
    expect(byId.get("group")?.disabled).toBe(true);
    expect(byId.get("duplicate")?.disabled).toBe(false);
  });
});
