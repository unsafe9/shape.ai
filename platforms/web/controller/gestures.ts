// Display formatting for the hold-key gesture catalog (the wasm core's `object_gesture_catalog()`):
// render one `HoldTrigger` into a read-only "Hold X" label for the settings modal.

import { detectMac, formatShortcut } from "./shortcuts";
import type { HoldTrigger } from "../bridge/sceneCoreWasm";

function capitalize(token: string): string {
  return token.length === 0 ? token : token[0].toUpperCase() + token.slice(1);
}

// Modifiers resolve through `formatShortcut` so `Mod`/`Shift`/`Alt` match the rest of the UI
// (⌘/⇧/⌥ on mac). A `degrees` parameter (the coarse-rotate step) is appended as `· 15°`.
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
