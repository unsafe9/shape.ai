import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import { emptyObjectScene, type Object as SceneObject, type ObjectScene, type ObjectSelection } from "../platforms/web/shared/object";
import {
  CANVAS_MENU,
  OBJECT_MENU,
  buildGroupOps,
  popOutPickEnabled,
  resolveContextMenuItems,
  resolveDoubleClick,
  ungroupPickEnabled
} from "../platforms/web/controller/interactions";
import { rectPathQuantized } from "../platforms/web/controller/transforms";

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

  it("buildGroupOps groups 1+ objects — one child grouped under a fresh frame is valid", () => {
    // A SINGLE object groups under a fresh frame (the `< 1` guard, not `< 2`).
    const one = geoObj("a", 100, 100);
    const { ops, frameId } = buildGroupOps(["a"], [one], { minX: 100, minY: 100, maxX: 180, maxY: 140 }, "frame-1", "z0", rectPathQuantized);
    expect(ops[0]).toMatchObject({ kind: "insert-object", object: { id: "frame-1" } });
    expect(ops.slice(1)).toEqual([{ kind: "reparent", id: "a", parent: "frame-1", order: "a0" }]);
    expect(frameId).toBe("frame-1");
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
