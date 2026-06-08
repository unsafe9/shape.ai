// AP3 (#9,#13,#18) — group / hierarchy / menus. Pins, at the code level (no
// renderer / no Svelte mount):
//  (a) group works on 1+ objects (the App.svelte guard is `< 1`, not `< 2`);
//  (b) the double-click drill-in branch (RA2b's signal): a container drills in, a
//      leaf enters text edit (the pure `doubleClickAction`), and App.svelte wires it
//      through `handleObjectDoubleClick` + `activeContainer`;
//  (c) ungroup is enabled ONLY for a container object (one WITH children) and
//      disabled for a childless leaf (`ungroupEnabled`);
//  (d) pop-out reparents a child to its grandparent / canvas root (`popOutOp`);
//  (f) the insert-text CANVAS_MENU entry + its handler are gone (D7).
// Falsifiable: any of these behaviors drifting (guard flips back, ungroup enabled
// for a leaf, pop-out lands at the wrong parent, insert-text returns) fails a case.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Object as SceneObject } from "../src/shared/object";
import { doubleClickAction, hasChildren, ungroupEnabled, popOutOp } from "../src/client/lib/grouping";

function obj(id: string, parent: string | undefined): SceneObject {
  return {
    id,
    ...(parent ? { parent } : {}),
    order: "a0",
    geometry: { d: "M 0 0 L 8 0" }
  } as SceneObject;
}

describe("doubleClickAction (RA2b drill-in branch, #9 / D6)", () => {
  it("drills into a container (hasChildren) and edits a leaf", () => {
    expect(doubleClickAction({ id: "frame", hasChildren: true })).toEqual({ kind: "drill-in", id: "frame" });
    expect(doubleClickAction({ id: "leaf", hasChildren: false })).toEqual({ kind: "edit-text", id: "leaf" });
  });

  it("is a no-op when the double-click hit no object", () => {
    expect(doubleClickAction(null)).toBeNull();
  });
});

describe("ungroupEnabled (#13)", () => {
  // A container (frame with a child) and a childless leaf.
  const scene = [obj("frame", undefined), obj("child", "frame"), obj("leaf", undefined)];

  it("is enabled for a container object (one WITH children)", () => {
    expect(hasChildren(scene, "frame")).toBe(true);
    expect(ungroupEnabled(scene, "frame")).toBe(true);
  });

  it("is disabled for a childless object", () => {
    expect(hasChildren(scene, "leaf")).toBe(false);
    expect(ungroupEnabled(scene, "leaf")).toBe(false);
  });

  it("is disabled when nothing is the single picked object (null id)", () => {
    expect(ungroupEnabled(scene, null)).toBe(false);
  });
});

describe("popOutOp (pop a child out one level, #18)", () => {
  // root frame -> mid frame -> deep child.
  const scene = [obj("root", undefined), obj("mid", "root"), obj("deep", "mid"), obj("top", undefined)];

  it("reparents a child to its GRANDPARENT", () => {
    expect(popOutOp(scene, "deep")).toEqual({ kind: "reparent", id: "deep", parent: "root", order: "a0" });
  });

  it("reparents to the canvas ROOT (absent parent) when the parent sits at the root", () => {
    // `mid`'s parent is `root`, whose parent is absent -> pop out to canvas root.
    expect(popOutOp(scene, "mid")).toEqual({ kind: "reparent", id: "mid", order: "a0" });
  });

  it("is a no-op for a root-level object (nothing to pop out of) or an unknown id", () => {
    expect(popOutOp(scene, "top")).toBeNull();
    expect(popOutOp(scene, "ghost")).toBeNull();
  });
});

describe("App.svelte wiring (AP3)", () => {
  const appSource = readFileSync(fileURLToPath(new URL("../src/client/svelte/App.svelte", import.meta.url)), "utf8");

  it("group works on 1+ objects — the guard is `< 1`, not `< 2`", () => {
    // The group guard must reject only the empty set.
    expect(appSource).toContain("if (ids.length < 1) return;");
    expect(appSource).not.toContain("if (ids.length < 2) return;");
  });

  it("branches double-click through the pure drill-in helper + active-container state", () => {
    expect(appSource).toContain("doubleClickAction");
    expect(appSource).toContain("handleObjectDoubleClick");
    expect(appSource).toContain("activeContainer");
  });

  it("gates ungroup + pop-out on the pure children/parent helpers", () => {
    expect(appSource).toContain("ungroupEnabled");
    expect(appSource).toContain("popOutOp");
    expect(appSource).toContain('id: "pop-out"');
  });

  it("drops the insert-text CANVAS_MENU entry AND its context-menu handler (D7)", () => {
    // The CANVAS_MENU entry is gone...
    expect(appSource).not.toContain('id: "insert-text"');
    // ...and so is the context-menu handler mapping (which anchored at `menu.world`).
    // The toolbar/shortcut `insert-text` handler (no anchor) is a separate map and stays.
    expect(appSource).not.toContain('insertPrimitive("text", menu.world)');
  });
});
