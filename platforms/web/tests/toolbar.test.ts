import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it, vi } from "vitest";
import {
  createShortcutDispatcher,
  isTypingTarget,
  matchCommand,
  parseShortcut
} from "../controller/shortcuts";
import { ensureSceneCore, loadSceneCore, type ObjectCommand } from "../bridge/sceneCoreWasm";
import { insertCommandToPrimitive, primitiveForCommand, primitiveOrder, toggleColorPopup, toggleStrokePopup, toolbarShapeKinds } from "../controller/toolbar";

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
    // clear-selection is a synthetic shell command, not in the wasm catalog.
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

describe("toolbar object-primitive mapping", () => {
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

describe("toolbar shape buttons", () => {
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

describe("toggleColorPopup (single toggle button)", () => {
  it("opens a closed popup and closes an open one (open->close on re-click)", () => {
    expect(toggleColorPopup(false)).toBe(true);
    expect(toggleColorPopup(true)).toBe(false);
  });
});

describe("toggleStrokePopup (Stroke toggle button)", () => {
  it("opens a closed popup and closes an open one (open->close on re-click)", () => {
    expect(toggleStrokePopup(false)).toBe(true);
    expect(toggleStrokePopup(true)).toBe(false);
  });

  it("is its own exported helper, independent of toggleColorPopup", () => {
    expect(typeof toggleStrokePopup).toBe("function");
    expect(toggleStrokePopup).not.toBe(toggleColorPopup);
  });
});

// The toolbar (incl. its Stroke/Color inline rows) is now rendered by the Rust `shape_ui` extension
// (crates/ui/src/toolbar.rs), so the popup/swatch/picker rendering is pinned by the crates/ui Rust tests,
// not against a .svelte source here. The thin-shell endstate guard asserts Toolbar.svelte no longer
// exists. What stays shell-side is the catalog->primitive mapping + the popup-toggle helpers (verified
// above) and the App-side Shift-driven recognition mode (below, read only from App.svelte).

describe("pen recognition mode (Basic default, Shift-hold Free)", () => {
  // No DOM in node: assert the wiring against the App source (the toolbar's Free-form toggle is gone —
  // the toolbar is Rust now — so only the App-side Shift-driven recognition mode is asserted here).
  const app = readFileSync(
    fileURLToPath(new URL("../ui/App.svelte", import.meta.url)),
    "utf8"
  );

  it("App drives Free recognition from a held Shift, defaulting to Basic", () => {
    expect(app).not.toMatch(/let freeRecognition\b/);
    expect(app).not.toContain("onToggleFreeRecognition");
    expect(app).toMatch(/let freeRecognitionHeld = \$state\(false\)/);
    // Routed through the catalog-backed predicate, not a raw event.shiftKey, so the
    // gesture self-documents in the settings modal.
    expect(app).toMatch(/freeRecognitionHeld = isFreeRecognizeHold\(event\)/);
    expect(app).toMatch(/freeRecognitionHeld \? "free" : "basic"/);
  });
});

describe("toolbar color contract (shell-side)", () => {
  it("App applies a picked color through applySelectedColor (the SelectColor intent target)", () => {
    // The Rust toolbar's Color swatch resolves to a SelectColor intent; the shell adopts it as the
    // next-shape default + recolors the selection through applySelectedColor (the preserved contract).
    const app = readFileSync(fileURLToPath(new URL("../ui/App.svelte", import.meta.url)), "utf8");
    expect(app).toContain('case "selectColor":');
    expect(app).toMatch(/applySelectedColor\(intent\.hex\)/);
  });
});
