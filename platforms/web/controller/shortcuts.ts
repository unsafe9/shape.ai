// Central shortcut dispatcher over the object command catalog (the wasm core's
// `object_command_catalog()`): map a keydown to a command id, then to a registered handler.
// Focus rule: shortcuts are ignored while typing in an input/textarea/select or contenteditable,
// EXCEPT a small allowlist that must always reach the app.

import type { ObjectCommand } from "../bridge/sceneCoreWasm";

export type CommandId = string;

export type CommandHandler = () => void;

export type ShortcutHandlers = Partial<Record<string, CommandHandler>>;

export type ParsedShortcut = {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
};

// Commands that must fire even while a text field is focused.
const FOCUS_EXEMPT_COMMANDS = new Set<string>(["clear-selection"]);

// True on macOS (decides whether `Mod` reads metaKey or ctrlKey).
export function detectMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return /mac|iphone|ipad|ipod/i.test(navigator.platform || navigator.userAgent || "");
}

// Parse a catalog shortcut string ("Mod+Shift+G", "Backspace", "]") into a normalized matcher.
// `Mod` is the platform primary modifier (Cmd/Ctrl); keys are lower-cased for case-insensitive matching.
export function parseShortcut(shortcut: string): ParsedShortcut {
  const tokens = shortcut.split("+");
  const result: ParsedShortcut = { key: "", mod: false, shift: false, alt: false };
  for (const token of tokens) {
    if (token === "Mod") result.mod = true;
    else if (token === "Shift") result.shift = true;
    else if (token === "Alt") result.alt = true;
    else result.key = normalizeKey(token);
  }
  return result;
}

function normalizeKey(key: string): string {
  if (key === " " || key === "Spacebar") return "space";
  if (key === "=" || key === "Plus") return "=";
  return key.toLowerCase();
}

// Resolve a KeyboardEvent to the command id whose binding it matches, or null. The first matching
// catalog entry wins (bindings are unique).
export function matchCommand(
  event: KeyboardEvent,
  catalog: ObjectCommand[],
  isMac = detectMac()
): CommandId | null {
  const eventKey = normalizeKey(event.key);
  const eventMod = isMac ? event.metaKey : event.ctrlKey;
  for (const command of catalog) {
    if (!command.defaultShortcut) continue;
    const parsed = parseShortcut(command.defaultShortcut);
    if (parsed.key !== eventKey) continue;
    if (parsed.mod !== eventMod) continue;
    if (parsed.shift !== event.shiftKey) continue;
    if (parsed.alt !== event.altKey) continue;
    return command.id;
  }
  return null;
}

// True when the event target is a text-entry surface that should swallow keys.
export function isTypingTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  if (!element) return false;
  const tag = element.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return element.isContentEditable === true;
}

// Render a default shortcut for display, resolving the `Mod` token to the host's primary modifier symbol.
export function formatShortcut(shortcut: string, isMac = detectMac()): string {
  return shortcut
    .split("+")
    .map((token) => {
      if (token === "Mod") return isMac ? "⌘" : "Ctrl";
      if (token === "Shift") return isMac ? "⇧" : "Shift";
      if (token === "Alt") return isMac ? "⌥" : "Alt";
      return token;
    })
    .join(isMac ? "" : "+");
}

export type ShortcutDispatcherOptions = {
  catalog: ObjectCommand[];
  handlers: ShortcutHandlers;
  isMac?: boolean;
};

// Build a keydown handler that resolves the event to a command and invokes the registered handler.
// Commands without a handler are ignored (no preventDefault). Returns the matched id (or null).
export function createShortcutDispatcher(options: ShortcutDispatcherOptions) {
  const isMac = options.isMac ?? detectMac();
  return function dispatch(event: KeyboardEvent): CommandId | null {
    const id = matchCommand(event, options.catalog, isMac);
    if (!id) return null;
    if (isTypingTarget(event.target) && !FOCUS_EXEMPT_COMMANDS.has(id)) return null;
    const handler = options.handlers[id];
    if (!handler) return null;
    event.preventDefault();
    handler();
    return id;
  };
}
