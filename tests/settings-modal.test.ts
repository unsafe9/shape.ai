// SM1 (#2) — the settings modal documents the hold-key gesture catalog.
//
// The Gestures section is fed by the wasm core's `object_gesture_catalog()` (C2,
// no TS mirror), loaded here via the real scene-core WASM. The node test env has
// no DOM, so we assert two things together: (1) the .svelte source renders the
// Gestures section against that catalog (label + trigger + description), and
// (2) `formatGestureTrigger` produces a readable trigger for every gesture,
// including coarse-rotate's 15-degree step. Falsifiable: dropping the Gestures
// section, or a gesture losing its label/trigger, fails the test.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import {
  ensureSceneCore,
  loadSceneCore,
  type ObjectGesture
} from "../platforms/web/bridge/sceneCoreWasm";
import { formatGestureTrigger } from "../platforms/web/controller/gestures";

let gestures: ObjectGesture[];

beforeAll(async () => {
  await ensureSceneCore();
  const core = await loadSceneCore();
  gestures = core.objectGestureCatalog();
});

const source = readFileSync(
  fileURLToPath(new URL("../platforms/web/ui/SettingsModal.svelte", import.meta.url)),
  "utf8"
);

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
});

describe("SettingsModal Gestures section (SM1, against the .svelte source)", () => {
  it("declares the gestures prop and a dedicated Gestures section", () => {
    expect(source).toMatch(/gestures:\s*ObjectGesture\[\]/);
    expect(source).toMatch(/<h3>Gestures<\/h3>/);
  });

  it("renders each gesture's label, formatted trigger, and description", () => {
    expect(source).toMatch(/\{gesture\.label\}/);
    expect(source).toMatch(/formatGestureTrigger\(gesture\.trigger,\s*isMac\)/);
    expect(source).toMatch(/\{gesture\.description\}/);
  });

  it("iterates the gesture catalog (keyed by id)", () => {
    expect(source).toMatch(/#each gestures as gesture \(gesture\.id\)/);
  });

  it("produces a non-empty label and trigger for every catalog gesture", () => {
    expect(gestures.length).toBeGreaterThan(0);
    for (const gesture of gestures) {
      expect(gesture.label.length).toBeGreaterThan(0);
      expect(formatGestureTrigger(gesture.trigger, false).length).toBeGreaterThan(0);
    }
  });
});
