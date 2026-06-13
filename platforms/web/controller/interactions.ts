// Interaction logic: each function takes every input explicitly (scene, selection, the scene-core
// handle, id/order factories) with no Svelte reactivity, DOM, or ambient state. Op construction
// lives in the Rust core (sceneCore); these compose the core's results into the shapes the shell authors.

import {
  GEOMETRY_QUANTUM_PER_PX,
  toggleObjectSelection,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type ObjectSelection,
  type Paint,
  type Transform3x3
} from "../shared/object";
import type { ObjectCommand, SceneCore } from "../bridge/sceneCoreWasm";
import { THEME_DEFAULT_COLOR, type DragSpan } from "./objectPrimitives";
import type { DragCreateShape, PrimitiveKindId } from "./toolbar";

// A plain pick on a member of the current Multi keeps the Multi (so a group-drag never collapses to
// a single); additive toggles the id in/out; everything else replaces the selection with that object.
export function routeSelectObject(current: ObjectSelection, id: string, additive: boolean): ObjectSelection {
  if (additive) return toggleObjectSelection(current, id);
  if (current.kind === "multi" && current.ids.includes(id)) return current;
  return { kind: "object", id };
}

// A marquee of >= 2 ids is a Multi, exactly 1 is a single object, 0 clears.
export function routeMarquee(ids: string[]): ObjectSelection {
  if (ids.length >= 2) return { kind: "multi", ids };
  if (ids.length === 1) return { kind: "object", id: ids[0] };
  return { kind: "canvas" };
}

// An Alt-held BODY translate of an anchored open-class single object is detachable (moves whole,
// clears its anchors). The class judgment is the core's; only single-root translate body drags qualify —
// a Multi drag anchored on a member (the whole-set move) is never a detach.
export function isDetachableBodyDrag(
  core: Pick<SceneCore, "isOpenClassD">,
  src: SceneObject,
  selection: ObjectSelection,
  id: string,
  kind: string,
  detach: boolean
): boolean {
  const singleRoot = !(selection.kind === "multi" && selection.ids.includes(id));
  return (
    detach &&
    kind === "translate" &&
    singleRoot &&
    (src.anchors?.length ?? 0) > 0 &&
    core.isOpenClassD(src.geometry.d ?? "")
  );
}

// The commit ops for a body drag — the Alt-detach composition when detachable, else the core's
// combined cascade + anchor-follow moveOpsForPick (which derives the MoveRoots from selection + picked
// id in-core) — collapsed to a single ObjectOp (one op stays bare; many batch).
export function commitBodyDrag(
  core: Pick<SceneCore, "isOpenClassD" | "detachMoveOps" | "moveOpsForPick">,
  scene: ObjectScene,
  selection: ObjectSelection,
  id: string,
  matrix: Transform3x3,
  kind: string,
  detach: boolean
): { op: ObjectOp; allOps: ObjectOp[] } {
  const src = scene.objects.find((o) => o.id === id);
  const allOps =
    src && isDetachableBodyDrag(core, src, selection, id, kind, detach)
      ? core.detachMoveOps(scene, id, matrix)
      : core.moveOpsForPick(scene, selection, id, matrix);
  const op: ObjectOp = allOps.length === 1 ? allOps[0] : { kind: "batch", ops: allOps };
  return { op, allOps };
}

// The dragged `id`'s composed transform from the cascade — its set-transform op (a multi cascade may
// put another member or a follow op first), falling back to its current transform. Drives the snap-back guard.
export function draggedRootTransform(
  allOps: ObjectOp[],
  id: string,
  fallback: SceneObject["transform"]
): SceneObject["transform"] {
  const op = allOps.find((o) => o.kind === "set-transform" && o.id === id);
  return op?.kind === "set-transform" ? op.transform : fallback;
}

// Honor an endpoint-drag snap only onto a REAL, OTHER canonical object (not the dragged id, not a
// transient preview); returns the hover-ring snap or null.
export function endpointSnapTarget(
  scene: ObjectScene,
  id: string,
  snapped: boolean,
  targetId: string | null
): string | null {
  return snapped && targetId !== null && targetId !== id && scene.objects.some((o) => o.id === targetId)
    ? targetId
    : null;
}

// The ONE undoable batch for an endpoint-drag release — the core's endpointReleaseOps (chord-deform
// edit-geometry + set-anchor rebind/unbind), collapsed to a single ObjectOp. Null on a no-op release.
export function endpointReleaseOp(
  core: Pick<SceneCore, "endpointReleaseOps">,
  scene: ObjectScene,
  id: string,
  nodeIndex: number,
  world: { x: number; y: number },
  target: string | null
): ObjectOp | null {
  const ops = core.endpointReleaseOps(scene, id, nodeIndex, world, target ? { targetId: target, at: world } : null);
  if (ops.length === 0) return null;
  return ops.length === 1 ? ops[0] : { kind: "batch", ops };
}

// Honor a snap ONLY when its target is a real canonical object (the transient drag-create preview /
// snap-indicator ids never are), so a self-snap onto the preview is dropped while a real-edge snap is kept.
export function canonicalizeCreateSnap(
  scene: ObjectScene,
  snapped: boolean,
  targetId: string | null
): { snapped: boolean; target: string | null } {
  const target = targetId !== null && scene.objects.some((o) => o.id === targetId) ? targetId : null;
  return { snapped: snapped && target !== null, target };
}

// The persistent pre-drag hover ring snap — set only when snapped onto a real canonical object, else null.
export function canonicalizeHoverSnap(
  scene: ObjectScene,
  snapped: boolean,
  targetId: string | null,
  world: { x: number; y: number }
): { at: { x: number; y: number }; target: string } | null {
  const target = targetId !== null && scene.objects.some((o) => o.id === targetId) ? targetId : null;
  return snapped && target !== null ? { at: world, target } : null;
}

// The immediate-insert object for `kind` at `center`, painted in `selectedColor`, built by the core.
export function buildInsertPrimitive(
  core: Pick<SceneCore, "buildPrimitive">,
  kind: PrimitiveKindId,
  center: { x: number; y: number },
  id: string,
  order: string,
  selectedColor: string
): SceneObject {
  return core.buildPrimitive(kind, center, id, order, selectedColor);
}

// The set-style op recoloring the single selected object to `color`, or null when nothing single is
// selected. The shell adopts the color regardless (it becomes the next-shape default); only a single selection recolors.
export function buildColorApplyOp(
  core: Pick<SceneCore, "buildSetStyleOp">,
  scene: ObjectScene,
  selection: ObjectSelection,
  color: string
): ObjectOp | null {
  if (selection.kind !== "object") return null;
  const object = scene.objects.find((o) => o.id === selection.id);
  if (!object) return null;
  return core.buildSetStyleOp(object, color);
}

// The double-click action — the core's container-vs-leaf decision for the signal's object; null signal is a no-op.
export type DoubleClickResolution =
  | { kind: "none" }
  | { kind: "drill-in"; id: string }
  | { kind: "edit-leaf"; id: string };

export function resolveDoubleClick(
  core: Pick<SceneCore, "doubleClickAction">,
  scene: ObjectScene,
  signal: { id: string; hasChildren: boolean } | null
): DoubleClickResolution {
  if (!signal) return { kind: "none" };
  const action = core.doubleClickAction(scene, signal.id);
  return action.kind === "drill-in-container" ? { kind: "drill-in", id: signal.id } : { kind: "edit-leaf", id: signal.id };
}

// A context-menu entry — a separator or a command row gated optionally on the picked target's shape
// (beyond the coarse `disabledFor` kind check). The icon is an opaque tag the shell maps to a component.
export type ContextMenuEntry<Icon = unknown> =
  | "separator"
  | {
      id: string;
      label?: string;
      icon?: Icon;
      danger?: boolean;
      disabledFor?: ObjectSelection["kind"];
      enabled?: (picked: ObjectSelection) => boolean;
    };

// Ungroup is enabled only for a single container object (has children).
export function ungroupPickEnabled(
  core: Pick<SceneCore, "ungroupEnabled">,
  scene: ObjectScene,
  picked: ObjectSelection
): boolean {
  return picked.kind === "object" && core.ungroupEnabled(scene, picked.id);
}

// Pop-out is enabled only when the single picked object has a parent.
export function popOutPickEnabled(
  core: Pick<SceneCore, "popOutOp">,
  scene: ObjectScene,
  picked: ObjectSelection
): boolean {
  return picked.kind === "object" && core.popOutOp(scene, picked.id) !== null;
}

// The object/multi menu. `pop-out` is a shell-only command (no catalog entry) so it carries its own label.
export const OBJECT_MENU: ContextMenuEntry<string>[] = [
  { id: "duplicate", icon: "copy" },
  { id: "group", icon: "group", disabledFor: "object" },
  { id: "ungroup", icon: "ungroup" },
  { id: "pop-out", label: "Pop out one level" },
  { id: "bring-to-front" },
  { id: "send-to-back" },
  { id: "add-comment", icon: "comment" },
  "separator",
  { id: "delete", icon: "trash", danger: true }
];

// The empty-canvas menu — quick inserts, the template library, and select-all.
export const CANVAS_MENU: ContextMenuEntry<string>[] = [
  { id: "insert-rectangle" },
  { id: "insert-ellipse" },
  { id: "open-template-library", icon: "template" },
  "separator",
  { id: "select-all" }
];

// A resolved menu item — label, disabled state (kind check OR the `enabled` predicate), icon tag,
// danger flag. `null` is a separator; an entry with no handler drops out (renders nothing).
export type ResolvedMenuItem<Icon = unknown> = {
  id: string;
  label: string;
  icon?: Icon;
  danger?: boolean;
  disabled: boolean;
};

export function resolveContextMenuItems<Icon>(
  picked: ObjectSelection,
  catalog: ObjectCommand[],
  hasHandler: (id: string) => boolean,
  enabledFor: (entry: Extract<ContextMenuEntry<Icon>, { id: string }>, picked: ObjectSelection) => boolean
): (ResolvedMenuItem<Icon> | null)[] {
  const layout = (picked.kind === "object" || picked.kind === "multi" ? OBJECT_MENU : CANVAS_MENU) as ContextMenuEntry<Icon>[];
  return layout.map((entry) => {
    if (entry === "separator") return null;
    if (!hasHandler(entry.id)) return null;
    const command = catalog.find((c) => c.id === entry.id);
    const disabled = entry.disabledFor === picked.kind || !enabledFor(entry, picked);
    return {
      id: entry.id,
      label: entry.label ?? command?.label ?? entry.id,
      icon: entry.icon,
      danger: entry.danger,
      disabled
    };
  });
}

// The renderer Paint for a transient preview's selected color — the theme-default sentinel renders
// as the "text" token, else solid. The canonical sentinel→Paint rule lives in the core; this is the
// per-frame-preview mirror, kept off the FFI hot path.
export function previewPaint(color: string): Paint {
  return color === THEME_DEFAULT_COLOR ? { kind: "token", name: "text" } : { kind: "solid", color };
}

const q = (px: number) => Math.round(px * GEOMETRY_QUANTUM_PER_PX);

// A transient preview object for the in-progress pen stroke — a world-px polyline (identity
// transform, so local==world) with the pen brush. Built in TS (NOT op-apply); the commit replaces it.
export function drawPreviewObject(
  points: { x: number; y: number }[],
  order: string,
  selectedColor: string,
  penWidthPx: number
): SceneObject {
  const d = points.map((p, i) => `${i === 0 ? "M" : "L"} ${q(p.x)} ${q(p.y)}`).join(" ");
  return {
    id: "draw-preview",
    order,
    transform: translateTransform(0, 0),
    geometry: { d, fillRule: "nonZero" },
    stroke: {
      paint: previewPaint(selectedColor),
      width: penWidthPx * GEOMETRY_QUANTUM_PER_PX,
      opacity: 1,
      cap: "round",
      join: "round"
    }
  };
}

// The transient rubber-band for one drag-create frame. The GEOMETRY (path + transform) comes from the
// core's build_primitive_from_drag — the same per-frame build the commit uses — so the preview never
// diverges from the committed object. The shell keeps only the transient id and overrides the paint to
// the preview appearance (no fill; a 2px butt/miter stroke painted via previewPaint), discarding the
// core primitive's default fill/stroke style.
export function createPreviewObject(
  core: Pick<SceneCore, "buildPrimitiveFromDrag">,
  kind: DragCreateShape,
  span: DragSpan,
  order: string,
  selectedColor: string
): SceneObject {
  const built = core.buildPrimitiveFromDrag(kind, span, "create-preview", order, selectedColor);
  return {
    ...built,
    fill: null,
    stroke: { paint: previewPaint(selectedColor), width: 2 * GEOMETRY_QUANTUM_PER_PX, opacity: 1, cap: "butt", join: "miter" }
  };
}

// A small ring drawn at a snapped corner so the user sees the snap. Its geometry is the core's ellipse
// over a 10x10 bbox centered on `at` (r=5), so the kappa-Bezier math lives only in the core; the shell
// keeps the transient id and the ring appearance (no fill; a 2px round/round #ff3b6b stroke).
const SNAP_RING_RADIUS = 5;
const SNAP_RING_COLOR = "#ff3b6b";
export function snapIndicatorObject(
  core: Pick<SceneCore, "buildPrimitiveFromDrag">,
  at: { x: number; y: number },
  order: string
): SceneObject {
  const r = SNAP_RING_RADIUS;
  const span: DragSpan = { start: { x: at.x - r, y: at.y - r }, end: { x: at.x + r, y: at.y + r } };
  const built = core.buildPrimitiveFromDrag("ellipse", span, "create-snap-indicator", order, SNAP_RING_COLOR);
  return {
    ...built,
    fill: null,
    stroke: { paint: { kind: "solid", color: SNAP_RING_COLOR }, width: 2 * GEOMETRY_QUANTUM_PER_PX, opacity: 1, cap: "round", join: "round" }
  };
}

// The renderer feed — the canonical scene plus any transient NEW-object preview (pen stroke /
// drag-create rubber-band + snap indicator, or the persistent pre-drag hover ring when no drag is
// in progress). Never mutates the canonical scene nor runs op-apply. The order factory puts every preview on top.
export function buildFeedScene(
  core: Pick<SceneCore, "buildPrimitiveFromDrag"> | null,
  source: ObjectScene,
  pen: { x: number; y: number }[] | null,
  create: DragCreateShape | null,
  createState: { span: DragSpan; snapped: boolean } | null,
  hoverSnap: { at: { x: number; y: number }; target: string | null } | null,
  nextOrder: () => string,
  selectedColor: string,
  penWidthPx: number
): ObjectScene {
  let feed = source;
  const preview = pen && pen.length >= 1 ? drawPreviewObject(pen, nextOrder(), selectedColor, penWidthPx) : null;
  if (preview) feed = { ...feed, objects: [...feed.objects, preview] };
  // The drag-create preview + snap ring source their geometry from the core; pre-load (core null) the
  // create path cannot commit anyway, so the transient feed simply omits them until the core is ready.
  if (core && create && createState) {
    const extra: SceneObject[] = [createPreviewObject(core, create, createState.span, nextOrder(), selectedColor)];
    if (createState.snapped) extra.push(snapIndicatorObject(core, createState.span.end, nextOrder()));
    feed = { ...feed, objects: [...feed.objects, ...extra] };
  } else if (core && hoverSnap) {
    // No drag in progress — render the persistent pre-drag anchor ring at the hovered edge.
    feed = { ...feed, objects: [...feed.objects, snapIndicatorObject(core, hoverSnap.at, nextOrder())] };
  }
  return feed;
}
