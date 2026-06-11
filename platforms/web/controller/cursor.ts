// EN1 (#4) — the affordance -> CSS cursor map, single-sourced for the unit gate.
//
// The core classifies the per-move hover into a `HoverAffordance` (W2-02); the
// shell maps each to a CSS cursor. The CSS lives in `styles.css` keyed by the
// `data-affordance` attribute on `.renderer-scene-surface`; this module is the
// canonical map the CSS mirrors, and the unit test pins both the map AND that
// `styles.css` carries the matching cursor for every affordance — so the #4 gap
// (a hardcoded `cursor: grab` on the topmost input canvas overriding the
// affordance cursor) cannot silently return.
//
// `cursorAffordance` is the App-level derivation: Space-hold forces the pan
// cursor; the crosshair tools own their own cursor (no affordance override);
// otherwise the core's hover affordance drives it. Pure + framework-neutral.

import type { HoverAffordance } from "../bridge/wasmLoader";
import type { ActiveTool } from "../renderer/engine";

/** The cursor wired to the `pan` hold gesture (Space / middle-button). */
export type CursorAffordance = HoverAffordance | "pan";

/**
 * The canonical affordance -> CSS `cursor` value. The keys cover every
 * `HoverAffordance` plus the synthetic `pan` (Space-hold). `styles.css` mirrors
 * this exactly via `[data-affordance="<key>"]`.
 */
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

/** The CSS cursor for an affordance (defaults to `default` for unknown values). */
export function affordanceToCursor(affordance: CursorAffordance): string {
  return AFFORDANCE_CURSOR[affordance] ?? "default";
}

/**
 * The App-level cursor affordance reflected onto the canvas wrapper's
 * `data-affordance`. Space-hold forces the pan grab cursor; the crosshair tools
 * (draw/create/erase) keep their own cursor (returns `null` so no override is
 * set); otherwise the core's per-move hover classification drives it.
 */
export function cursorAffordance(
  spaceHeld: boolean,
  activeTool: ActiveTool,
  affordance: HoverAffordance
): CursorAffordance | null {
  if (spaceHeld) return "pan";
  if (activeTool === "draw" || activeTool === "create" || activeTool === "erase") return null;
  return affordance;
}
