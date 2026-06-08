// SM1 (#2) — display formatting for the hold-key gesture catalog.
//
// The catalog is the wasm core's `object_gesture_catalog()` (C2: no TS mirror).
// `formatGestureTrigger` renders one `HoldTrigger` into a read-only "Hold X"
// label for the settings modal. Pure; display-only. Kept framework-neutral (like
// `formatShortcut`) so the test exercises it without mounting a Svelte tree.

import { detectMac, formatShortcut } from "./shortcuts";
import type { HoldTrigger } from "../scene/sceneCoreWasm";

/** Title-case a single bare token (e.g. `middle` -> `Middle`). */
function capitalize(token: string): string {
  return token.length === 0 ? token : token[0].toUpperCase() + token.slice(1);
}

/**
 * Render a gesture's hold-trigger for display. Modifiers resolve through
 * `formatShortcut` so `Mod`/`Shift`/`Alt` match the rest of the UI (⌘/⇧/⌥ on
 * mac). A `degrees` parameter (the coarse-rotate step) is appended as `· 15°`.
 */
export function formatGestureTrigger(trigger: HoldTrigger, isMac = detectMac()): string {
  let token: string;
  switch (trigger.input) {
    case "key":
      token = trigger.key ?? "";
      break;
    case "button":
      token = `${capitalize(trigger.button ?? "")} Button`;
      break;
    case "modifier":
      token = formatShortcut(trigger.modifier ?? "", isMac);
      break;
  }
  const base = `Hold ${token}`;
  return trigger.degrees != null ? `${base} · ${trigger.degrees}°` : base;
}
