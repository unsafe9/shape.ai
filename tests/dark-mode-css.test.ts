// Pins that every shell surface inverts to a near-black palette in dark mode by
// asserting the resolved CSS-variable graph (not a substring): the dark block must
// override the surface base/text/canvas to genuinely dark values, and the key
// floating surfaces must consume that themeable base so they flip with it.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("../platforms/web/styles.css", import.meta.url)), "utf8");

// Extract `--name: value;` declarations from the first matching block.
function readVars(source: string, selector: string): Map<string, string> {
  const start = source.indexOf(selector + " {");
  if (start < 0) throw new Error(`block not found: ${selector}`);
  const open = source.indexOf("{", start);
  const close = source.indexOf("}", open);
  // Strip block comments first so a `--var` after an inline comment still parses.
  const body = source.slice(open + 1, close).replace(/\/\*[\s\S]*?\*\//g, "");
  const vars = new Map<string, string>();
  for (const decl of body.split(";")) {
    const m = decl.match(/^\s*(--[\w-]+)\s*:\s*(.+?)\s*$/s);
    if (m) vars.set(m[1], m[2]);
  }
  return vars;
}

// Resolve a var() expression against a variable map (a few levels of nesting).
function resolve(map: Map<string, string>, expr: string): string {
  let out = expr;
  for (let i = 0; i < 4 && out.includes("var("); i += 1) {
    out = out.replace(/var\((--[\w-]+)\)/g, (_, name) => map.get(name) ?? `var(${name})`);
  }
  return out;
}

// Approximate sRGB luminance (0..255) of a `#rrggbb` or `rgb(r g b[ / a])`.
function luminance(color: string): number {
  let r = 0;
  let g = 0;
  let b = 0;
  const hex = color.match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    r = parseInt(hex[1].slice(0, 2), 16);
    g = parseInt(hex[1].slice(2, 4), 16);
    b = parseInt(hex[1].slice(4, 6), 16);
  } else {
    const rgb = color.match(/rgb\(\s*(\d+)\s+(\d+)\s+(\d+)/);
    if (!rgb) throw new Error(`cannot parse color: ${color}`);
    [r, g, b] = [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];
  }
  return 0.2126 * r + 0.7152 * g + 0.587 * b;
}

const light = readVars(css, ":root");
const dark = readVars(css, ':root[data-theme="dark"]');

describe("dark-mode CSS variable graph", () => {
  it("flips the surface base from white to near-black (and they differ)", () => {
    expect(luminance("rgb(" + light.get("--surface-rgb")! + ")")).toBeGreaterThan(220);
    expect(luminance("rgb(" + dark.get("--surface-rgb")! + ")")).toBeLessThan(60);
    expect(dark.get("--surface-rgb")).not.toBe(light.get("--surface-rgb"));
  });

  it("flips text to light-on-dark (text luminance inverts)", () => {
    expect(luminance(light.get("--text")!)).toBeLessThan(80);
    expect(luminance(dark.get("--text")!)).toBeGreaterThan(180);
  });

  it("flips the root/canvas backgrounds to dark", () => {
    for (const name of ["--bg", "--canvas", "--surface-solid"] as const) {
      expect(luminance(light.get(name)!)).toBeGreaterThan(180);
      expect(luminance(dark.get(name)!)).toBeLessThan(70);
    }
  });

  it("resolves the composed --surface token to a near-black surface in dark mode", () => {
    // Resolving the graph proves the composed surface inherits the dark base, not
    // just the raw triplet.
    const resolved = resolve(dark, dark.get("--surface")!);
    expect(resolved).not.toContain("var(");
    expect(luminance(resolved)).toBeLessThan(70);
  });

  it("collapses the faint light tint wash onto a dark base in dark mode", () => {
    expect(luminance("rgb(" + light.get("--surface-tint-rgb")! + ")")).toBeGreaterThan(190);
    expect(luminance("rgb(" + dark.get("--surface-tint-rgb")! + ")")).toBeLessThan(70);
  });
});

describe("every key floating surface consumes the themeable base (so it flips)", () => {
  // Capture each surface's `background:` and assert it resolves (through the dark
  // map) to a near-black color, i.e. wired to the theme, not a hardcoded value.
  function surfaceBackground(selector: string): string {
    const start = css.indexOf(selector + " {");
    expect(start, `selector ${selector} present`).toBeGreaterThanOrEqual(0);
    const open = css.indexOf("{", start);
    const close = css.indexOf("}", open);
    const body = css.slice(open + 1, close);
    const m = body.match(/background:\s*([^;]+);/);
    expect(m, `${selector} has a background`).not.toBeNull();
    return m![1].trim();
  }

  for (const selector of [".toolbar-remote", ".toolbar-draw", ".settings-modal", ".node-context-menu", ".template-picker", ".color-popup"]) {
    it(`${selector} resolves to a dark surface in dark mode`, () => {
      const bg = surfaceBackground(selector);
      const resolved = resolve(dark, bg);
      expect(resolved, `${selector} background still references an unresolved var`).not.toContain("var(--surface");
      // First color token of the (possibly gradient) background.
      const firstColor = resolved.match(/#[0-9a-f]{6}|rgb\([^)]*\)/i);
      expect(firstColor, `${selector} resolved background has a color`).not.toBeNull();
      expect(luminance(firstColor![0]), `${selector} -> ${firstColor![0]}`).toBeLessThan(70);
    });
  }
});
