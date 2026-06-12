// The affordance -> CSS cursor map. `styles.css` mirrors it via `[data-affordance="<key>"]`,
// and the unit test pins both the map AND that styles.css carries a cursor for every affordance.

import type { HoverAffordance } from "../bridge/wasmLoader";
import type { ActiveTool } from "../renderer/engine";

// The cursor wired to the `pan` hold gesture (Space / middle-button).
export type CursorAffordance = HoverAffordance | "pan";

// Keys cover every `HoverAffordance` plus the synthetic `pan` (Space-hold).
export const AFFORDANCE_CURSOR: Record<CursorAffordance, string> = {
  empty: "default",
  body: "move",
  "resize-nw": "nwse-resize",
  "resize-se": "nwse-resize",
  "resize-ne": "nesw-resize",
  "resize-sw": "nesw-resize",
  "resize-n": "ns-resize",
  "resize-s": "ns-resize",
  "resize-e": "ew-resize",
  "resize-w": "ew-resize",
  rotate: "crosshair",
  pan: "grab"
};

export function affordanceToCursor(affordance: CursorAffordance): string {
  return AFFORDANCE_CURSOR[affordance] ?? "default";
}

// The cursor affordance reflected onto the canvas wrapper's `data-affordance`. Space-hold forces
// the pan grab cursor; the crosshair tools (draw/create/erase) return `null` (no override).
export function cursorAffordance(
  spaceHeld: boolean,
  activeTool: ActiveTool,
  affordance: HoverAffordance
): CursorAffordance | null {
  if (spaceHeld) return "pan";
  if (activeTool === "draw" || activeTool === "create" || activeTool === "erase") return null;
  return affordance;
}
