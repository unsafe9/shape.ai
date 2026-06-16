// The Gestures section is fed by the wasm core's object_gesture_catalog() (single
// source, no TS mirror). The settings modal is now rendered by the Rust `shape_ui`
// extension (crates/ui/src/settings.rs), so the section-rendering wiring is pinned by
// the crates/ui Rust tests, not a .svelte source here. What stays shell-side and is
// verified below: the catalog itself (from the real core) and formatGestureTrigger,
// the shell helper the modal's trigger formatting and the settings catalog both read.

import { beforeAll, describe, expect, it } from "vitest";
import {
  ensureSceneCore,
  loadSceneCore,
  type ObjectGesture
} from "../bridge/sceneCoreWasm";
import { formatGestureTrigger } from "../controller/gestures";

let gestures: ObjectGesture[];

beforeAll(async () => {
  await ensureSceneCore();
  const core = await loadSceneCore();
  gestures = core.objectGestureCatalog();
});

describe("object gesture catalog (from the wasm core)", () => {
  it("is non-empty with id/label/trigger/description rows", () => {
    expect(gestures.length).toBeGreaterThan(0);
    for (const gesture of gestures) {
      expect(typeof gesture.id).toBe("string");
      expect(typeof gesture.label).toBe("string");
      expect(typeof gesture.description).toBe("string");
      expect(typeof gesture.trigger.input).toBe("string");
    }
  });

  it("carries the coarse-rotate 15-degree step", () => {
    const coarse = gestures.find((g) => g.id === "coarse-rotate-shift");
    expect(coarse?.trigger.degrees).toBe(15);
  });
});

describe("formatGestureTrigger", () => {
  it("renders the key/button/modifier hold-trigger families", () => {
    expect(formatGestureTrigger({ input: "key", key: "Space" }, false)).toBe("Hold Space");
    expect(formatGestureTrigger({ input: "button", button: "middle" }, false)).toBe(
      "Hold Middle Button"
    );
    expect(formatGestureTrigger({ input: "modifier", modifier: "Shift" }, false)).toBe("Hold Shift");
  });

  it("appends the degrees parameter (coarse-rotate's 15 deg)", () => {
    expect(
      formatGestureTrigger({ input: "modifier", modifier: "Shift", degrees: 15 }, false)
    ).toBe("Hold Shift · 15°");
  });

  it("produces a non-empty label and trigger for every catalog gesture", () => {
    expect(gestures.length).toBeGreaterThan(0);
    for (const gesture of gestures) {
      expect(gesture.label.length).toBeGreaterThan(0);
      expect(formatGestureTrigger(gesture.trigger, false).length).toBeGreaterThan(0);
    }
  });
});
