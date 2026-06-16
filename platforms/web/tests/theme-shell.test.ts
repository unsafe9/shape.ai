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
} from "../renderer/scene";

// Mirror of scene-core `object::theme::ALL_TOKENS` (not wasm-exported to JS); a
// drift in either direction fails the equality below.
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

describe("applyDocumentTheme (toggle)", () => {
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

  it("a toggle click flips the theme state AND drives setObjectTheme with the new dark bit", () => {
    // The live handler: `toggleTheme()` flips the reactive theme, then the $effect runs
    // `applyDocumentTheme`, which calls `host.setObjectTheme(dark)`. This reproduces that
    // exact flip+apply round-trip (the model the Rust-side dead-click defect blocked) and
    // asserts each click both flips the persisted theme and drives the renderer setter.
    // FAILS if a click does not flip the theme or does not push the new dark bit.
    const root = fakeRoot();
    const storage = fakeStorage();
    const driven: boolean[] = [];
    // The reactive theme state + the App's toggleTheme/$effect, isolated from Svelte.
    let theme: Theme = readStoredTheme(storage); // "light" by default
    const toggleTheme = () => {
      theme = theme === "dark" ? "light" : "dark";
      applyDocumentTheme(theme, { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    };

    toggleTheme();
    expect(theme).toBe("dark");
    expect(storage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(root.get(THEME_ROOT_ATTRIBUTE)).toBe("dark");
    expect(driven).toEqual([true]);

    toggleTheme();
    expect(theme).toBe("light");
    expect(storage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(driven).toEqual([true, false]);
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
    fileURLToPath(new URL("../ui/App.svelte", import.meta.url)),
    "utf8"
  );
  const cssSource = readFileSync(
    fileURLToPath(new URL("../styles.css", import.meta.url)),
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

  it("applies the persisted theme to the renderer the moment the host is wired", () => {
    // REGRESSION GUARD: the theme $effect runs at mount when `host` is still null (a
    // non-reactive `let`, so it never re-runs on assignment). Without applying the theme
    // in the host-wiring path too, the renderer never hears it and dark mode renders light
    // (light UI fills under a dark canvas, near-white labels on white buttons). `handleHost`
    // MUST drive setObjectTheme with the current theme. Fails if that call is removed.
    const start = appSource.indexOf("function handleHost");
    expect(start).toBeGreaterThan(-1);
    const handleHostBody = appSource.slice(start, start + 900);
    expect(handleHostBody).toContain('host.setObjectTheme(theme === "dark")');
  });
});
