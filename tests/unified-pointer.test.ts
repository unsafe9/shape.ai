// W2-03 — the unified Move pointer's two pure shell classifications:
//  - pan-intent: which pointer-down is a pan vs a pick/marquee, and
//  - selection-toggle: how Shift+click grows/shrinks the multi-select set.
// Both are framework-neutral so they can be pinned without a renderer or a Svelte
// mount (the engine/App only wire them).

import { describe, expect, it } from "vitest";
import { isPanIntent } from "../src/client/renderer/engine";
import { toggleObjectSelection, type ObjectSelection } from "../src/shared/object";

describe("isPanIntent (W2-03 pan classification)", () => {
  it("middle button always pans, regardless of Space", () => {
    expect(isPanIntent({ spaceHeld: false, button: 1 })).toBe(true);
    expect(isPanIntent({ spaceHeld: true, button: 1 })).toBe(true);
  });

  it("left button pans only while Space is held", () => {
    expect(isPanIntent({ spaceHeld: true, button: 0 })).toBe(true);
    expect(isPanIntent({ spaceHeld: false, button: 0 })).toBe(false);
  });

  it("right button never pans (it is the context menu)", () => {
    expect(isPanIntent({ spaceHeld: false, button: 2 })).toBe(false);
    expect(isPanIntent({ spaceHeld: true, button: 2 })).toBe(false);
  });
});

describe("toggleObjectSelection (W2-03 multi-select set)", () => {
  it("adds an object to an empty (canvas) selection", () => {
    expect(toggleObjectSelection({ kind: "canvas" }, "a")).toEqual({ kind: "object", id: "a" });
  });

  it("grows a single selection into a multi when a second id is added", () => {
    expect(toggleObjectSelection({ kind: "object", id: "a" }, "b")).toEqual({ kind: "multi", ids: ["a", "b"] });
  });

  it("removes a member from a multi and collapses to object when one remains", () => {
    const multi: ObjectSelection = { kind: "multi", ids: ["a", "b"] };
    expect(toggleObjectSelection(multi, "a")).toEqual({ kind: "object", id: "b" });
  });

  it("removing the only selected object clears to canvas", () => {
    expect(toggleObjectSelection({ kind: "object", id: "a" }, "a")).toEqual({ kind: "canvas" });
  });

  it("keeps existing order and appends the newly toggled id at the end", () => {
    const multi: ObjectSelection = { kind: "multi", ids: ["a", "b", "c"] };
    expect(toggleObjectSelection(multi, "d")).toEqual({ kind: "multi", ids: ["a", "b", "c", "d"] });
    expect(toggleObjectSelection(multi, "b")).toEqual({ kind: "multi", ids: ["a", "c"] });
  });
});
