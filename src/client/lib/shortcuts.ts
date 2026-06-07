// CC0.4 / CC6.1 — central shortcut dispatcher.
//
// Loads the command catalog (the TS mirror of scene-core's command_catalog) and
// maps a keydown to a command id, then to a registered handler. Keeping this in
// lib/ (framework-neutral) means the cockpit test can exercise the parse/match
// logic without mounting a Svelte tree, and the shell only registers handlers.
//
// Focus rule: shortcuts are ignored while typing in an input/textarea/select or
// a contenteditable region, EXCEPT a small allowlist (Escape) that must always
// reach the app. This mirrors the existing App.svelte keydown gate.

import { commandCatalog, detectMac, type Command } from "./commandCatalog";

export type CommandId = Command["id"];

/** A handler runs the side effect for a command; return value is ignored. */
export type CommandHandler = () => void;

export type ShortcutHandlers = Partial<Record<string, CommandHandler>>;

/** Parsed binding: a normalized key plus required modifier flags. */
export type ParsedShortcut = {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
};

// Commands that must fire even while a text field is focused.
const FOCUS_EXEMPT_COMMANDS = new Set<string>(["clear-selection"]);

/**
 * Parse a catalog shortcut string ("Mod+Shift+G", "Backspace", "]") into a
 * normalized matcher. `Mod` is the platform primary modifier (Cmd/Ctrl). Keys
 * are lower-cased so matching is case-insensitive; symbol keys pass through.
 */
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

/** Normalize an event/binding key token for comparison. */
function normalizeKey(key: string): string {
  if (key === " " || key === "Spacebar") return "space";
  if (key === "=" || key === "Plus") return "=";
  return key.length === 1 ? key.toLowerCase() : key.toLowerCase();
}

/**
 * Resolve a KeyboardEvent to the command id whose binding it matches, or null.
 * `isMac` decides whether `Mod` reads metaKey (Cmd) or ctrlKey. The first
 * catalog entry whose binding matches wins (the catalog has unique bindings).
 */
export function matchCommand(event: KeyboardEvent, isMac = detectMac()): CommandId | null {
  const eventKey = normalizeKey(event.key);
  const eventMod = isMac ? event.metaKey : event.ctrlKey;
  // The non-primary control key on mac is ctrl, which we don't bind, so ignore
  // it; on non-mac, meta (Win/Cmd) is likewise unbound.
  for (const command of commandCatalog) {
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

/** True when the event target is a text-entry surface that should swallow keys. */
export function isTypingTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  if (!element) return false;
  const tag = element.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return element.isContentEditable === true;
}

export type ShortcutDispatcherOptions = {
  handlers: ShortcutHandlers;
  isMac?: boolean;
};

/**
 * Build a keydown handler that resolves the event to a command and invokes the
 * registered handler. Commands without a handler are ignored (no preventDefault),
 * so unhandled keys keep their browser behavior. Returns the matched id (or null)
 * so callers/tests can assert dispatch without observing the side effect.
 */
export function createShortcutDispatcher(options: ShortcutDispatcherOptions) {
  const isMac = options.isMac ?? detectMac();
  return function dispatch(event: KeyboardEvent): CommandId | null {
    const id = matchCommand(event, isMac);
    if (!id) return null;
    if (isTypingTarget(event.target) && !FOCUS_EXEMPT_COMMANDS.has(id)) return null;
    const handler = options.handlers[id];
    if (!handler) return null;
    event.preventDefault();
    handler();
    return id;
  };
}
