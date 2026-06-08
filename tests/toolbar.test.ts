// U4 — toolbar shortcut dispatch + object-primitive mapping.
//
// The shortcut dispatcher matches a keydown against the object command catalog
// (the wasm core's object_command_catalog(), loaded here via the real scene-core
// WASM — P1, no TS mirror) and invokes a registered handler. The toolbar's
// insert-* commands map to object-primitive kinds.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it, vi } from "vitest";
import {
  createShortcutDispatcher,
  isTypingTarget,
  matchCommand,
  parseShortcut
} from "../src/client/lib/shortcuts";
import { ensureSceneCore, loadSceneCore, type ObjectCommand } from "../src/client/scene/sceneCoreWasm";
import { insertCommandToPrimitive, primitiveForCommand, primitiveOrder, toggleColorPopup, toggleStrokePopup, toolbarShapeKinds } from "../src/client/lib/toolbar";

let catalog: ObjectCommand[];

beforeAll(async () => {
  await ensureSceneCore();
  const core = await loadSceneCore();
  catalog = core.objectCommandCatalog();
});

function keyEvent(init: {
  key: string;
  code?: string;
  meta?: boolean;
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
  target?: unknown;
}): KeyboardEvent {
  return {
    key: init.key,
    code: init.code ?? "",
    metaKey: init.meta ?? false,
    ctrlKey: init.ctrl ?? false,
    shiftKey: init.shift ?? false,
    altKey: init.alt ?? false,
    target: init.target ?? null,
    preventDefault: vi.fn()
  } as unknown as KeyboardEvent;
}

describe("object command catalog (from the wasm core)", () => {
  it("is non-empty with id/label/category rows", () => {
    expect(catalog.length).toBeGreaterThan(0);
    for (const command of catalog) {
      expect(typeof command.id).toBe("string");
      expect(typeof command.label).toBe("string");
      expect(typeof command.category).toBe("string");
    }
  });

  it("has unique command ids", () => {
    const ids = catalog.map((command) => command.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("carries undo / redo with their bindings", () => {
    expect(catalog.find((c) => c.id === "undo")?.defaultShortcut).toBe("Mod+Z");
    expect(catalog.find((c) => c.id === "redo")?.defaultShortcut).toBe("Mod+Shift+Z");
  });
});

describe("parseShortcut", () => {
  it("parses a Mod+Shift combo", () => {
    expect(parseShortcut("Mod+Shift+G")).toEqual({ key: "g", mod: true, shift: true, alt: false });
  });

  it("parses a bare key", () => {
    expect(parseShortcut("V")).toEqual({ key: "v", mod: false, shift: false, alt: false });
  });

  it("parses symbol keys", () => {
    expect(parseShortcut("[").key).toBe("[");
    expect(parseShortcut("Mod+=").key).toBe("=");
  });
});

describe("matchCommand (against the object catalog)", () => {
  it("maps Backspace to delete", () => {
    expect(matchCommand(keyEvent({ key: "Backspace" }), catalog, true)).toBe("delete");
  });

  it("resolves Mod to Cmd on mac and Ctrl elsewhere", () => {
    expect(matchCommand(keyEvent({ key: "a", meta: true }), catalog, true)).toBe("select-all");
    expect(matchCommand(keyEvent({ key: "a", ctrl: true }), catalog, false)).toBe("select-all");
    expect(matchCommand(keyEvent({ key: "a", meta: true }), catalog, false)).toBeNull();
  });

  it("requires the exact modifier set (Mod+Z is undo, Mod+Shift+Z is redo)", () => {
    expect(matchCommand(keyEvent({ key: "z", meta: true }), catalog, true)).toBe("undo");
    expect(matchCommand(keyEvent({ key: "z", meta: true, shift: true }), catalog, true)).toBe("redo");
  });

  it("returns null for unbound keys", () => {
    expect(matchCommand(keyEvent({ key: "q", meta: true }), catalog, true)).toBeNull();
  });
});

describe("isTypingTarget", () => {
  it("treats input/textarea/select as typing surfaces", () => {
    expect(isTypingTarget({ tagName: "INPUT", isContentEditable: false } as unknown as HTMLElement)).toBe(true);
    expect(isTypingTarget({ tagName: "TEXTAREA", isContentEditable: false } as unknown as HTMLElement)).toBe(true);
    expect(isTypingTarget({ tagName: "SELECT", isContentEditable: false } as unknown as HTMLElement)).toBe(true);
  });

  it("treats contenteditable regions as typing surfaces", () => {
    expect(isTypingTarget({ tagName: "DIV", isContentEditable: true } as unknown as HTMLElement)).toBe(true);
  });

  it("treats plain elements as non-typing", () => {
    expect(isTypingTarget({ tagName: "DIV", isContentEditable: false } as unknown as HTMLElement)).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });
});

describe("createShortcutDispatcher", () => {
  it("invokes the registered handler and preventDefault for a matched command", () => {
    const onDelete = vi.fn();
    const dispatch = createShortcutDispatcher({ catalog, isMac: true, handlers: { delete: onDelete } });
    const event = keyEvent({ key: "Backspace" });
    expect(dispatch(event)).toBe("delete");
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
  });

  it("ignores commands with no registered handler (no preventDefault)", () => {
    const dispatch = createShortcutDispatcher({ catalog, isMac: true, handlers: {} });
    const event = keyEvent({ key: "Backspace" });
    expect(dispatch(event)).toBeNull();
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it("suppresses shortcuts while typing in an input", () => {
    const onDelete = vi.fn();
    const dispatch = createShortcutDispatcher({ catalog, isMac: true, handlers: { delete: onDelete } });
    const event = keyEvent({ key: "Backspace", target: { tagName: "INPUT", isContentEditable: false } });
    expect(dispatch(event)).toBeNull();
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("still allows clear-selection (Escape) while typing", () => {
    // clear-selection is a synthetic shell command (not in the wasm catalog); add
    // it to the dispatcher's catalog to assert the focus-exempt path.
    const clear = vi.fn();
    const withEscape: ObjectCommand[] = [
      ...catalog,
      { id: "clear-selection", label: "Clear", category: "selection", defaultShortcut: "Escape", description: "" }
    ];
    const dispatch = createShortcutDispatcher({ catalog: withEscape, isMac: true, handlers: { "clear-selection": clear } });
    const event = keyEvent({ key: "Escape", target: { tagName: "TEXTAREA", isContentEditable: false } });
    expect(dispatch(event)).toBe("clear-selection");
    expect(clear).toHaveBeenCalledTimes(1);
  });
});

describe("toolbar object-primitive mapping (U1)", () => {
  it("maps the exact insert-* primitive ids", () => {
    expect(primitiveForCommand("insert-rectangle")).toBe("rectangle");
    expect(primitiveForCommand("insert-ellipse")).toBe("ellipse");
    expect(primitiveForCommand("insert-line")).toBe("line");
    expect(primitiveForCommand("insert-text")).toBe("text");
    expect(primitiveForCommand("insert-frame")).toBe("frame");
  });

  it("returns null for non-shape commands", () => {
    expect(primitiveForCommand("select-all")).toBeNull();
    expect(primitiveForCommand("undo")).toBeNull();
  });

  it("covers the same primitive set as the toolbar order", () => {
    expect(new Set(Object.values(insertCommandToPrimitive))).toEqual(new Set(primitiveOrder));
  });
});

describe("toolbar shape buttons (TB1 / D7)", () => {
  it("shows only rect/ellipse/line buttons in order", () => {
    expect(toolbarShapeKinds).toEqual(["rectangle", "ellipse", "line"]);
  });

  it("drops the frame and text shape buttons (authored via shortcut/context-menu only)", () => {
    expect(toolbarShapeKinds).not.toContain("frame");
    expect(toolbarShapeKinds).not.toContain("text");
  });

  it("keeps text/frame in the insert command mapping (shortcut + context-menu survive)", () => {
    expect(primitiveForCommand("insert-text")).toBe("text");
    expect(primitiveForCommand("insert-frame")).toBe("frame");
  });
});

describe("toggleColorPopup (TB1 / #3 — single toggle button)", () => {
  it("opens a closed popup and closes an open one (open->close on re-click)", () => {
    expect(toggleColorPopup(false)).toBe(true); // closed -> open
    expect(toggleColorPopup(true)).toBe(false); // open  -> closed (re-click closes)
  });
});

describe("toggleStrokePopup (S1 / #4 — Stroke toggle button)", () => {
  it("opens a closed popup and closes an open one (open->close on re-click)", () => {
    expect(toggleStrokePopup(false)).toBe(true); // closed -> open
    expect(toggleStrokePopup(true)).toBe(false); // open  -> closed (re-click closes)
  });

  it("is its own exported helper, independent of toggleColorPopup", () => {
    expect(typeof toggleStrokePopup).toBe("function");
    expect(toggleStrokePopup).not.toBe(toggleColorPopup);
  });
});

describe("toolbar Stroke popup UI (S1 / #4)", () => {
  // No DOM in the node test env: assert the wiring against the .svelte source.
  // Falsifiable — re-adding the auto draw sub-toolbar, dropping the toggle button,
  // or losing the size/color controls inside the popup all fail these.
  const source = readFileSync(
    fileURLToPath(new URL("../src/client/svelte/Toolbar.svelte", import.meta.url)),
    "utf8"
  );

  it("drops the auto draw sub-toolbar (no activeTool === draw|erase gate)", () => {
    expect(source).not.toMatch(/activeTool === "draw" \|\| activeTool === "erase"/);
    expect(source).not.toMatch(/class="toolbar-draw"/);
  });

  it("renders a Stroke button that toggles its popup", () => {
    expect(source).toMatch(/aria-label="Stroke"/);
    expect(source).toMatch(/onclick=\{toggleStrokePopupOpen\}/);
    expect(source).toMatch(/aria-expanded=\{strokePopupOpen\}/);
    expect(source).toMatch(/\{#if strokePopupOpen\}/);
  });

  it("the Stroke popup holds the brush size buttons AND the brush color swatches", () => {
    const popup = source.slice(source.indexOf('aria-label="Stroke settings"'));
    expect(popup).toMatch(/\{#each penWidths as width/);
    expect(popup).toMatch(/onclick=\{\(\)\s*=>\s*onSetPenWidth\(width\)\}/);
    expect(popup).toMatch(/\{#each penPalette as color/);
    expect(popup).toMatch(/onclick=\{\(\)\s*=>\s*onSetPenColor\(color\)\}/);
  });

  it("closes on outside-click and Escape while open (its own effect)", () => {
    expect(source).toMatch(/strokePopupOpen\s*=\s*false/);
    expect(source).toMatch(/strokeControl && !strokeControl\.contains/);
  });
});

describe("toolbar color-popup UI (TB1 / #3)", () => {
  // The node test env has no DOM, so we assert the component's prop/callback
  // contract and the popup wiring against the .svelte source. Falsifiable: dropping
  // the single toggle button, the popup-gated swatches, the native <input
  // type="color">, the selectedColor prop, or the onSelectColor wiring fails this.
  const source = readFileSync(
    fileURLToPath(new URL("../src/client/svelte/Toolbar.svelte", import.meta.url)),
    "utf8"
  );

  it("declares the selectedColor prop and onSelectColor callback (preserved contract)", () => {
    expect(source).toMatch(/selectedColor:\s*string;/);
    expect(source).toMatch(/onSelectColor:\s*\(color:\s*string\)\s*=>\s*void;/);
  });

  it("renders a single color-trigger button (rainbow swatch) that toggles the popup", () => {
    expect(source).toMatch(/class="icon-button color-trigger/);
    expect(source).toMatch(/onclick=\{toggleColorPopupOpen\}/);
    expect(source).toMatch(/aria-expanded=\{colorPopupOpen\}/);
    // The popup (and therefore the swatches + picker) is gated behind the open
    // state — it is NOT an always-visible row.
    expect(source).toMatch(/\{#if colorPopupOpen\}/);
  });

  it("the popup exposes BOTH the fixed PEN_PALETTE swatches AND the native picker", () => {
    expect(source).toMatch(/class="color-popup"/);
    expect(source).toMatch(/\{#each penPalette as color/);
    expect(source).toMatch(/type="color"/);
  });

  it("selecting either a swatch or the picker updates selectedColor via onSelectColor", () => {
    expect(source).toMatch(/onclick=\{\(\)\s*=>\s*onSelectColor\(color\)\}/);
    expect(source).toMatch(/oninput=\{\(event\)\s*=>\s*onSelectColor\(/);
  });

  it("closes on outside-click and Escape while open", () => {
    expect(source).toMatch(/colorPopupOpen\s*=\s*false/);
    expect(source).toMatch(/event\.key\s*===\s*"Escape"/);
  });
});
