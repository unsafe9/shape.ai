// AP2 (#15) — parent-drag cascade.
//
// A drag commits a world-space delta matrix for the dragged object. Children are
// reparented under a frame with world-absolute transforms (D3), so moving the
// parent must apply the SAME world-space delta to every descendant — otherwise a
// frame slides out from under its contents. This is pure matrix math (no scene
// access, no op-apply), so the shell test can pin the cascade.

import { IDENTITY_TRANSFORM, type Object as SceneObject, type ObjectOp, type Transform3x3 } from "../../shared/object";

/**
 * W2-05: 3x3 row-major pre-multiply newTransform = delta * base, with an absent
 * base treated as the identity. The delta is the cumulative world-space gesture
 * matrix; the base is the object's existing transform.
 */
export function composeTransform(delta: Transform3x3, base: SceneObject["transform"]): Transform3x3 {
  const b = base ?? IDENTITY_TRANSFORM;
  const out: Transform3x3 = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0]
  ];
  for (let r = 0; r < 3; r++) {
    for (let c = 0; c < 3; c++) {
      out[r][c] = delta[r][0] * b[0][c] + delta[r][1] * b[1][c] + delta[r][2] * b[2][c];
    }
  }
  return out;
}

/**
 * The `set-transform` ops a drag of `id` produces: the dragged object first, then
 * every descendant (transitively, via the `parent` chain), each carrying the same
 * world-space `delta` composed onto its own base. Order is parent-before-child so
 * the batch is deterministic. Returns `[]` when `id` is not in the scene.
 */
export function cascadeTransformOps(objects: SceneObject[], id: string, delta: Transform3x3): ObjectOp[] {
  const root = objects.find((o) => o.id === id);
  if (!root) return [];
  const ops: ObjectOp[] = [{ kind: "set-transform", id, transform: composeTransform(delta, root.transform) }];
  // BFS the parent chain so a child that is itself a frame cascades to its own
  // children. Each descendant is visited once (the parent graph is a forest).
  const frontier = [id];
  while (frontier.length > 0) {
    const parent = frontier.shift()!;
    for (const child of objects) {
      if (child.parent !== parent) continue;
      ops.push({ kind: "set-transform", id: child.id, transform: composeTransform(delta, child.transform) });
      frontier.push(child.id);
    }
  }
  return ops;
}

/**
 * AP2 (#10): a Multi selection drags as one unit. The renderer anchors the gesture
 * on a single picked id, but the same world-space `delta` applies to EVERY selected
 * member (and each member's subtree, via {@link cascadeTransformOps}). Unions the
 * per-member cascades, deduping by id so an object that is both a selected member
 * and a descendant of another member is transformed once. Order is member-input
 * order, parent-before-child within each subtree. Returns `[]` when no id is live.
 */
export function cascadeMultiTransformOps(objects: SceneObject[], ids: string[], delta: Transform3x3): ObjectOp[] {
  const ops: ObjectOp[] = [];
  const seen = new Set<string>();
  for (const id of ids) {
    for (const op of cascadeTransformOps(objects, id, delta)) {
      const opId = op.kind === "set-transform" ? op.id : "";
      if (seen.has(opId)) continue;
      seen.add(opId);
      ops.push(op);
    }
  }
  return ops;
}
