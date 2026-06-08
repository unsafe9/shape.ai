// AP3 (#9,#13,#18) — group hierarchy decisions.
//
// The shell's group/ungroup/pop-out/drill-in behavior reduces to a few pure
// decisions over the object forest: who has children, where a popped-out child
// reparents, and whether a double-click drills into a container or edits a leaf.
// These hold no scene access, randomness, or op-apply, so the shell test pins
// them directly (mirrors transformCascade.ts).

import type { Object as SceneObject, ObjectOp, ObjectId } from "../../shared/object";

/** RA2b double-click drill-in signal (mirrors the core `ObjectDoubleClick`). */
export type ObjectDoubleClick = { id: string; hasChildren: boolean };

/** The shell's response to a double-click: drill into a container, or edit a leaf. */
export type DoubleClickAction = { kind: "drill-in"; id: string } | { kind: "edit-text"; id: string };

/**
 * Branch RA2b's double-click signal: an object WITH children is a container, so
 * the shell drills in (sets active-container state, AP3); a leaf enters inline
 * text edit (the existing path). A null signal (no object double-clicked) is no-op.
 */
export function doubleClickAction(signal: ObjectDoubleClick | null): DoubleClickAction | null {
  if (!signal) return null;
  return signal.hasChildren ? { kind: "drill-in", id: signal.id } : { kind: "edit-text", id: signal.id };
}

/** Whether `id` is a container (has at least one child) in the object forest. */
export function hasChildren(objects: SceneObject[], id: ObjectId): boolean {
  return objects.some((o) => o.parent === id);
}

/**
 * Ungroup is enabled ONLY for a single selected object that is a container (has
 * children). A childless object, a multi-select, or the canvas is not ungroupable.
 */
export function ungroupEnabled(objects: SceneObject[], selectedId: ObjectId | null): boolean {
  return selectedId !== null && hasChildren(objects, selectedId);
}

/**
 * Pop a child out one level: reparent it to its parent's parent (the grandparent),
 * or to the canvas root (absent `parent`) when the parent sits at the root. Returns
 * null when `id` is unknown or already at the root (nothing to pop out of). The
 * child keeps its order key — pop-out is a containment change, not a reorder.
 */
export function popOutOp(objects: SceneObject[], id: ObjectId): ObjectOp | null {
  const child = objects.find((o) => o.id === id);
  if (!child || child.parent === undefined) return null;
  const parent = objects.find((o) => o.id === child.parent);
  const grandparent = parent?.parent;
  return grandparent === undefined
    ? { kind: "reparent", id, order: child.order }
    : { kind: "reparent", id, parent: grandparent, order: child.order };
}
