// The single source for the hold-key gesture bindings the pointer pipeline keys behavior off.
// The canonical catalog is the wasm core's `object_gesture_catalog()`, but the engine's hot-path
// handlers cannot await a wasm round-trip per event, so the FROZEN binding token per behavior is
// mirrored here; `verifyGestureBindings` proves the mirror stays equal to the runtime catalog (gated).

import type { ObjectGesture } from "../bridge/sceneCoreWasm";

export const GESTURE_PAN_SPACE = "pan-space";
export const GESTURE_PAN_MIDDLE = "pan-middle";
export const GESTURE_ADDITIVE_SELECT_SHIFT = "additive-select-shift";
export const GESTURE_ADDITIVE_SELECT_MOD = "additive-select-mod";
export const GESTURE_NO_SNAP_ALT = "no-snap-alt";
export const GESTURE_PARTIAL_ERASE_ALT = "partial-erase-alt";
export const GESTURE_COARSE_ROTATE_SHIFT = "coarse-rotate-shift";
export const GESTURE_DETACH_ALT = "detach-alt";
export const GESTURE_FREE_RECOGNIZE_SHIFT = "free-recognize-shift";

// The frozen binding token per routed gesture. `key`/`button`/`modifier` carry the held input;
// `degrees` the optional numeric parameter (the coarse-rotate step).
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

// DOM MouseEvent button value for the middle mouse button.
export const MIDDLE_MOUSE_BUTTON = 1;

export const COARSE_ROTATE_SNAP_DEG = GESTURE_BINDINGS[GESTURE_COARSE_ROTATE_SHIFT].degrees;

export type GestureModifiers = {
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
};

// The `Mod` token resolves to metaKey (Cmd) on macOS, else ctrlKey.
function modHeld(event: GestureModifiers, isMac: boolean): boolean {
  return isMac ? event.metaKey : event.ctrlKey;
}

// A pointer-down is a pan (not a pick/marquee) when the middle button is used OR the left button
// is used with Space held. `button` follows DOM MouseEvent values (0=left, 1=middle).
export function isPanGesture(intent: { spaceHeld: boolean; button: number }): boolean {
  if (intent.button === MIDDLE_MOUSE_BUTTON) return true;
  return intent.button === 0 && intent.spaceHeld;
}

// Shift or the platform primary modifier (Cmd/Ctrl) held at pick time toggles the hit into the
// multi-select set instead of replacing it.
export function isAdditiveSelect(event: GestureModifiers, isMac: boolean): boolean {
  return event.shiftKey || modHeld(event, isMac);
}

export function isSnapBypass(event: { altKey: boolean }): boolean {
  return event.altKey;
}

export function isPartialErase(event: { altKey: boolean }): boolean {
  return event.altKey;
}

// Inverted: rotation snaps to the catalog step (15°) BY DEFAULT; Shift rotates freely (fine). This
// predicate reports "Shift held"; the shell negates it at the call site.
export function isCoarseRotate(event: { shiftKey: boolean }): boolean {
  return event.shiftKey;
}

export function isDetachDrag(event: { altKey: boolean }): boolean {
  return event.altKey;
}

// Shift held while drawing with the pen recognizes a free-form shape instead of snapping to a basic primitive.
export function isFreeRecognizeHold(event: { shiftKey: boolean }): boolean {
  return event.shiftKey;
}

// Returns the routed binding ids that drifted from the runtime catalog entry for the same id (or
// are missing from it); empty means the mirror is current. The unit gate asserts this is empty.
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
