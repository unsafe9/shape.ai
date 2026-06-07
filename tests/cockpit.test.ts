import { describe, expect, it, vi } from "vitest";
import {
  createShortcutDispatcher,
  isTypingTarget,
  matchCommand,
  parseShortcut
} from "../src/client/lib/shortcuts";
import { commandCatalog } from "../src/client/lib/commandCatalog";
import { insertCommandToPrimitive, primitiveForCommand, primitiveOrder } from "../src/client/lib/cockpitCommands";

// A minimal KeyboardEvent stand-in: createShortcutDispatcher / matchCommand only
// read key/metaKey/ctrlKey/shiftKey/altKey/target plus preventDefault.
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

describe("shortcut catalog", () => {
  it("has unique command ids", () => {
    const ids = commandCatalog.map((command) => command.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("has unique bindings per resolved platform", () => {
    for (const isMac of [true, false]) {
      const seen = new Map<string, string>();
      for (const command of commandCatalog) {
        if (!command.defaultShortcut) continue;
        const parsed = parseShortcut(command.defaultShortcut);
        const signature = `${parsed.key}|${parsed.mod}|${parsed.shift}|${parsed.alt}`;
        expect(seen.has(signature), `binding collision ${command.id} vs ${seen.get(signature)} (mac=${isMac})`).toBe(false);
        seen.set(signature, command.id);
      }
    }
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

describe("matchCommand", () => {
  it("maps a bare letter to its tool command", () => {
    expect(matchCommand(keyEvent({ key: "v" }), true)).toBe("select-move");
    expect(matchCommand(keyEvent({ key: "h" }), true)).toBe("hand-pan");
  });

  it("resolves Mod to Cmd on mac and Ctrl elsewhere", () => {
    expect(matchCommand(keyEvent({ key: "a", meta: true }), true)).toBe("select-all");
    expect(matchCommand(keyEvent({ key: "a", ctrl: true }), false)).toBe("select-all");
    // Cmd on a non-mac host is not the primary modifier → no match.
    expect(matchCommand(keyEvent({ key: "a", meta: true }), false)).toBeNull();
  });

  it("requires the exact modifier set (Mod+Shift+G is distinct from Mod+G)", () => {
    expect(matchCommand(keyEvent({ key: "g", meta: true }), true)).toBe("group");
    expect(matchCommand(keyEvent({ key: "g", meta: true, shift: true }), true)).toBe("ungroup");
  });

  it("matches the settings binding Mod+,", () => {
    expect(matchCommand(keyEvent({ key: ",", meta: true }), true)).toBe("open-settings");
  });

  it("returns null for unbound keys", () => {
    expect(matchCommand(keyEvent({ key: "z", meta: true }), true)).toBeNull();
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
    const insertRectangle = vi.fn();
    const dispatch = createShortcutDispatcher({ isMac: true, handlers: { "insert-rectangle": insertRectangle } });
    const event = keyEvent({ key: "r" });
    expect(dispatch(event)).toBe("insert-rectangle");
    expect(insertRectangle).toHaveBeenCalledTimes(1);
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
  });

  it("ignores commands with no registered handler (no preventDefault)", () => {
    const dispatch = createShortcutDispatcher({ isMac: true, handlers: {} });
    const event = keyEvent({ key: "r" });
    expect(dispatch(event)).toBeNull();
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it("suppresses shortcuts while typing in an input", () => {
    const insertRectangle = vi.fn();
    const dispatch = createShortcutDispatcher({ isMac: true, handlers: { "insert-rectangle": insertRectangle } });
    const event = keyEvent({ key: "r", target: { tagName: "INPUT", isContentEditable: false } });
    expect(dispatch(event)).toBeNull();
    expect(insertRectangle).not.toHaveBeenCalled();
  });

  it("still allows clear-selection (Escape) while typing", () => {
    const clear = vi.fn();
    const dispatch = createShortcutDispatcher({ isMac: true, handlers: { "clear-selection": clear } });
    const event = keyEvent({ key: "Escape", target: { tagName: "TEXTAREA", isContentEditable: false } });
    expect(dispatch(event)).toBe("clear-selection");
    expect(clear).toHaveBeenCalledTimes(1);
  });
});

describe("remote insertion mapping", () => {
  it("maps every insert-* shape command to a primitive", () => {
    const shapeCommands = commandCatalog.filter((command) => command.category === "shape");
    for (const command of shapeCommands) {
      const primitive = primitiveForCommand(command.id);
      expect(primitive, `no primitive for ${command.id}`).not.toBeNull();
    }
  });

  it("maps the exact catalog shape ids", () => {
    expect(primitiveForCommand("insert-rectangle")).toBe("rectangle");
    expect(primitiveForCommand("insert-ellipse")).toBe("ellipse");
    expect(primitiveForCommand("insert-connector")).toBe("connector");
    expect(primitiveForCommand("insert-sticky")).toBe("sticky");
    expect(primitiveForCommand("insert-frame")).toBe("frame");
  });

  it("returns null for non-shape commands", () => {
    expect(primitiveForCommand("select-move")).toBeNull();
    expect(primitiveForCommand("zoom-in")).toBeNull();
  });

  it("covers the same primitive set as the remote order", () => {
    expect(new Set(Object.values(insertCommandToPrimitive))).toEqual(new Set(primitiveOrder));
  });
});
