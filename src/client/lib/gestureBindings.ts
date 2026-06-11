// EN1 (#2-route/#3-shell) — the single source for the hold-key gesture bindings
// the pointer pipeline keys behavior off.
//
// The canonical gesture catalog is the wasm core's `object_gesture_catalog()`
// (C2: no TS mirror of the catalog itself). But the engine's pointer/keyboard
// handlers run on the hot path and cannot await a wasm round-trip per event, so
// the FROZEN binding token for each behavior is mirrored here as a constant —
// and `verifyGestureBindings` proves this mirror stays equal to the runtime C2
// catalog (a falsifiable single-source check, run in the unit gate). The engine
// routes Space-pan / additive-select / no-snap / partial-erase / coarse-rotate
// through these predicates instead of magic literals.
//
// Pure + framework-neutral so the test pins the routing without a renderer.

import type { ObjectGesture } from "../scene/sceneCoreWasm";

/** The frozen C2 gesture ids the shell routes pointer behavior off. */
export const GESTURE_PAN_SPACE = "pan-space";
export const GESTURE_PAN_MIDDLE = "pan-middle";
export const GESTURE_ADDITIVE_SELECT_SHIFT = "additive-select-shift";
export const GESTURE_ADDITIVE_SELECT_MOD = "additive-select-mod";
export const GESTURE_NO_SNAP_ALT = "no-snap-alt";
export const GESTURE_PARTIAL_ERASE_ALT = "partial-erase-alt";
export const GESTURE_COARSE_ROTATE_SHIFT = "coarse-rotate-shift";
export const GESTURE_DETACH_ALT = "detach-alt";
export const GESTURE_FREE_RECOGNIZE_SHIFT = "free-recognize-shift";

/**
 * The frozen binding token for each routed gesture, mirrored from C2. `key`/
 * `button`/`modifier` carry the held input; `degrees` the optional numeric
 * parameter (the coarse-rotate step). `verifyGestureBindings` pins these to the
 * runtime catalog so a drift in C2 fails the gate rather than silently skewing
 * the routing.
 */
export const GESTURE_BINDINGS = {
  [GESTURE_PAN_SPACE]: { key: "Space" },
  [GESTURE_PAN_MIDDLE]: { button: "middle" },
  [GESTURE_ADDITIVE_SELECT_SHIFT]: { modifier: "Shift" },
  [GESTURE_ADDITIVE_SELECT_MOD]: { modifier: "Mod" },
  [GESTURE_NO_SNAP_ALT]: { modifier: "Alt" },
  [GESTURE_PARTIAL_ERASE_ALT]: { modifier: "Alt" },
  [GESTURE_COARSE_ROTATE_SHIFT]: { modifier: "Shift", degrees: 15 },
  [GESTURE_DETACH_ALT]: { modifier: "Alt" },
  [GESTURE_FREE_RECOGNIZE_SHIFT]: { modifier: "Shift" }
} as const;

/** DOM MouseEvent button value for the middle mouse button (C2 `pan-middle`). */
export const MIDDLE_MOUSE_BUTTON = 1;

/** The coarse-rotate snap step in degrees (C2 `coarse-rotate-shift`). */
export const COARSE_ROTATE_SNAP_DEG = GESTURE_BINDINGS[GESTURE_COARSE_ROTATE_SHIFT].degrees;

/** The flags an input event exposes, pared to what the gesture predicates read. */
export type GestureModifiers = {
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
};

/** True on macOS, so the `Mod` token resolves to metaKey (Cmd) vs ctrlKey. */
function modHeld(event: GestureModifiers, isMac: boolean): boolean {
  return isMac ? event.metaKey : event.ctrlKey;
}

/**
 * A pointer-down is a pan gesture (C2 `pan-space` / `pan-middle`) — not a pick/
 * marquee — when the middle mouse button is used OR the left button is used with
 * Space held. `button` follows the DOM MouseEvent values (0=left, 1=middle).
 */
export function isPanGesture(intent: { spaceHeld: boolean; button: number }): boolean {
  if (intent.button === MIDDLE_MOUSE_BUTTON) return true;
  return intent.button === 0 && intent.spaceHeld;
}

/**
 * The additive-select gesture (C2 `additive-select-shift` / `-mod`): Shift or the
 * platform primary modifier (Cmd/Ctrl) held at pick time, so the shell toggles
 * the hit into the multi-select set instead of replacing it.
 */
export function isAdditiveSelect(event: GestureModifiers, isMac: boolean): boolean {
  return event.shiftKey || modHeld(event, isMac);
}

/** The snap-bypass gesture (C2 `no-snap-alt`): Alt held suppresses snapping. */
export function isSnapBypass(event: { altKey: boolean }): boolean {
  return event.altKey;
}

/**
 * The partial-erase gesture (C2 `partial-erase-alt`): Alt held with the erase
 * tool cuts partial geometry instead of deleting the whole object.
 */
export function isPartialErase(event: { altKey: boolean }): boolean {
  return event.altKey;
}

/**
 * The coarse-rotate gesture (C2 `coarse-rotate-shift`), inverted: rotation snaps
 * to the catalog step (15°) BY DEFAULT, and holding Shift rotates freely (fine).
 * This predicate still reports "Shift held"; the shell negates it at the call
 * site, so no Shift = snap and Shift = the fine override.
 */
export function isCoarseRotate(event: { shiftKey: boolean }): boolean {
  return event.shiftKey;
}

/**
 * The detach gesture (C2 `detach-alt`, anchor-semantics v3 §3/DU4): Alt held
 * while dragging an anchored object moves it whole and detaches its anchors.
 */
export function isDetachDrag(event: { altKey: boolean }): boolean {
  return event.altKey;
}

/**
 * The free-form-recognition gesture (C2 `free-recognize-shift`): Shift held while
 * drawing with the pen recognizes the stroke as a free-form shape instead of
 * snapping it to a basic primitive (released = back to Basic).
 */
export function isFreeRecognizeHold(event: { shiftKey: boolean }): boolean {
  return event.shiftKey;
}

/**
 * Falsifiable single-source check: every routed binding above equals the runtime
 * C2 catalog entry for the same id (and the catalog still carries all routed
 * ids). Returns the mismatched ids; an empty array means the mirror is current.
 * The unit gate asserts this is empty, so a C2 drift fails the build, not the UX.
 */
export function verifyGestureBindings(catalog: ObjectGesture[]): string[] {
  const byId = new Map(catalog.map((gesture) => [gesture.id, gesture]));
  const mismatched: string[] = [];
  for (const [id, binding] of Object.entries(GESTURE_BINDINGS)) {
    const gesture = byId.get(id);
    if (!gesture) {
      mismatched.push(id);
      continue;
    }
    const trigger = gesture.trigger;
    const expected = binding as { key?: string; button?: string; modifier?: string; degrees?: number };
    if (
      trigger.key !== expected.key ||
      trigger.button !== expected.button ||
      trigger.modifier !== expected.modifier ||
      trigger.degrees !== expected.degrees
    ) {
      mismatched.push(id);
    }
  }
  return mismatched;
}
