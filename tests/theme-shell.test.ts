// AP4 (#12c) — theme shell. The dark/light toggle must (a) flip the root
// `data-theme` attribute (dark-mode CSS), (b) persist the choice, and (c) drive
// RB1's renderer theme-bit so the canvas + chrome flip together; and the shell's
// default object styles must reference C1 token names. These pin the pure
// scene.ts theme helpers plus assert the App.svelte / styles.css wiring without a
// renderer or a Svelte mount. Falsifiable: a toggle that skips the root attr,
// persistence, or the renderer setter — or a default-style token ref that drifts
// off the C1 set — fails the test.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  applyDocumentTheme,
  readStoredTheme,
  isTheme,
  THEME_TOKEN_NAMES,
  THEME_STORAGE_KEY,
  THEME_ROOT_ATTRIBUTE,
  DEFAULT_OBJECT_STYLE_TOKENS,
  type Theme
} from "../src/client/renderer/scene";

// The C1 contract token names (scene-core `object::theme::ALL_TOKENS`). The C1
// table is not wasm-exported to JS, so the contract is mirrored here; a drift in
// either direction fails the equality below.
const C1_TOKEN_NAMES = [
  "canvas-bg",
  "surface",
  "surface-muted",
  "default-fill",
  "default-stroke",
  "text",
  "shadow",
  "selection-ring"
];

function fakeRoot() {
  const attrs = new Map<string, string>();
  return {
    setAttribute: (name: string, value: string) => attrs.set(name, value),
    get: (name: string) => attrs.get(name)
  };
}

function fakeStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value)
  };
}

describe("applyDocumentTheme (AP4 toggle)", () => {
  it("flips the root data-theme attribute, persists, AND drives the renderer bit", () => {
    const root = fakeRoot();
    const storage = fakeStorage();
    const driven: boolean[] = [];

    applyDocumentTheme("dark", { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    expect(root.get(THEME_ROOT_ATTRIBUTE)).toBe("dark");
    expect(storage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(driven).toEqual([true]);

    applyDocumentTheme("light", { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    expect(root.get(THEME_ROOT_ATTRIBUTE)).toBe("light");
    expect(storage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(driven).toEqual([true, false]);
  });

  it("is a safe no-op renderer-side when no setter is supplied (renderer not yet ready)", () => {
    const root = fakeRoot();
    const storage = fakeStorage();
    expect(() => applyDocumentTheme("dark", { root, storage })).not.toThrow();
    expect(root.get(THEME_ROOT_ATTRIBUTE)).toBe("dark");
  });
});

describe("readStoredTheme / isTheme", () => {
  it("round-trips the persisted choice and defaults to light", () => {
    const storage = fakeStorage();
    expect(readStoredTheme(storage)).toBe("light");
    storage.setItem(THEME_STORAGE_KEY, "dark");
    expect(readStoredTheme(storage)).toBe("dark");
    storage.setItem(THEME_STORAGE_KEY, "garbage");
    expect(readStoredTheme(storage)).toBe("light");
  });

  it("recognizes only the two valid themes", () => {
    const themes: Theme[] = ["light", "dark"];
    for (const t of themes) expect(isTheme(t)).toBe(true);
    expect(isTheme("garbage")).toBe(false);
    expect(isTheme(null)).toBe(false);
  });
});

describe("default object style tokens reference the C1 token set", () => {
  it("THEME_TOKEN_NAMES equals the C1 contract names", () => {
    expect([...THEME_TOKEN_NAMES].sort()).toEqual([...C1_TOKEN_NAMES].sort());
  });

  it("every default-style token ref exists in C1", () => {
    for (const name of Object.values(DEFAULT_OBJECT_STYLE_TOKENS)) {
      expect(C1_TOKEN_NAMES).toContain(name);
    }
    expect(DEFAULT_OBJECT_STYLE_TOKENS.fill).toBe("default-fill");
    expect(DEFAULT_OBJECT_STYLE_TOKENS.stroke).toBe("default-stroke");
    expect(DEFAULT_OBJECT_STYLE_TOKENS.text).toBe("text");
  });
});

describe("App.svelte + styles.css wiring", () => {
  const appSource = readFileSync(
    fileURLToPath(new URL("../src/client/svelte/App.svelte", import.meta.url)),
    "utf8"
  );
  const cssSource = readFileSync(
    fileURLToPath(new URL("../src/client/styles.css", import.meta.url)),
    "utf8"
  );

  it("drives the toggle through applyDocumentTheme with BOTH the root attr and the renderer setter", () => {
    expect(appSource).toContain("applyDocumentTheme");
    expect(appSource).toContain("document.documentElement");
    expect(appSource).toContain("setObjectTheme");
    expect(appSource).toContain("toggleTheme");
  });

  it("ships dark-mode CSS keyed on the root data-theme attribute", () => {
    expect(cssSource).toContain('[data-theme="dark"]');
    expect(cssSource).toContain("color-scheme: dark");
  });
});
