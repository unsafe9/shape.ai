import { describe, expect, it } from "vitest";
import { isPanIntent } from "../platforms/web/renderer/engine";
import { toggleObjectSelection, type ObjectSelection } from "../platforms/web/shared/object";

describe("isPanIntent (pan classification)", () => {
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

describe("toggleObjectSelection (multi-select set)", () => {
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
