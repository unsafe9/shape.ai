// Framework-neutral interaction logic lifted out of App.svelte's god-controller.
//
// Each function takes every input explicitly (scene, selection, the scene-core
// handle, id/order factories...) — no Svelte reactivity, no DOM, no ambient
// state — so App.svelte composes them through thin reactive wrappers and the unit
// gate pins them by exercising the real behavior (op shapes, routing decisions,
// menu items) without a mount. Op construction itself still lives in the Rust core
// (sceneCore); these compose the core's results into the shapes the shell authors.

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
import type { MoveRoots, ObjectCommand, SceneCore } from "../bridge/sceneCoreWasm";
import { THEME_DEFAULT_COLOR, altDetachOps, type DragSpan } from "./objectPrimitives";
import type { DragCreateShape, PrimitiveKindId } from "./toolbar";

// ----- selection routing (onSelectObject / onMarquee) -----

// W3-G5 (#10): a plain pick on a member of the current Multi keeps the Multi (so a
// group-drag never collapses to a single object); additive toggles the id in/out;
// everything else replaces the selection with that object.
export function routeSelectObject(current: ObjectSelection, id: string, additive: boolean): ObjectSelection {
  if (additive) return toggleObjectSelection(current, id);
  if (current.kind === "multi" && current.ids.includes(id)) return current;
  return { kind: "object", id };
}

// RA2a: a marquee of >= 2 ids is a Multi, exactly 1 is a single object, 0 clears.
export function routeMarquee(ids: string[]): ObjectSelection {
  if (ids.length >= 2) return { kind: "multi", ids };
  if (ids.length === 1) return { kind: "object", id: ids[0] };
  return { kind: "canvas" };
}

// ----- body-drag commit (onTransformCommit) -----

// AP2 (#10): a Multi drag anchored on the picked `id` moves EVERY member; otherwise
// the dragged single root cascades to its own subtree (the core's moveOps).
export function moveRootsFor(selection: ObjectSelection, id: string): MoveRoots {
  return selection.kind === "multi" && selection.ids.includes(id)
    ? { kind: "multi", ids: selection.ids }
    : { kind: "single", id };
}

// v3 §3 (DU4): an Alt-held BODY translate of an anchored open-class single object is
// detachable — it moves whole and clears its anchors. The class judgment is the
// core's (isOpenClassD); only single-root translate body drags qualify.
export function isDetachableBodyDrag(
  core: Pick<SceneCore, "isOpenClassD">,
  src: SceneObject,
  roots: MoveRoots,
  kind: string,
  detach: boolean
): boolean {
  return (
    detach &&
    kind === "translate" &&
    roots.kind === "single" &&
    (src.anchors?.length ?? 0) > 0 &&
    core.isOpenClassD(src.geometry.d ?? "")
  );
}

// Tier-2: the commit ops for a body drag — the Alt-detach composition when
// detachable, else the core's combined cascade + anchor-follow moveOps — collapsed
// to a single ObjectOp (a one-op result stays bare; many ops wrap in a batch).
export function commitBodyDrag(
  core: Pick<SceneCore, "isOpenClassD" | "moveOps">,
  scene: ObjectScene,
  selection: ObjectSelection,
  id: string,
  matrix: Transform3x3,
  kind: string,
  detach: boolean
): { op: ObjectOp; allOps: ObjectOp[] } {
  const src = scene.objects.find((o) => o.id === id);
  const roots = moveRootsFor(selection, id);
  const allOps =
    src && isDetachableBodyDrag(core, src, roots, kind, detach)
      ? altDetachOps(core, scene, id, matrix)
      : core.moveOps(scene, roots, matrix);
  const op: ObjectOp = allOps.length === 1 ? allOps[0] : { kind: "batch", ops: allOps };
  return { op, allOps };
}

// FC-16: the dragged `id`'s composed transform from the cascade — read ITS
// set-transform op (a multi cascade may put another member first / a follow op at
// [0]), falling back to its current transform. Drives the pendingCommit snap-back
// guard keyed on the dragged id.
export function draggedRootTransform(
  allOps: ObjectOp[],
  id: string,
  fallback: SceneObject["transform"]
): SceneObject["transform"] {
  const op = allOps.find((o) => o.kind === "set-transform" && o.id === id);
  return op?.kind === "set-transform" ? op.transform : fallback;
}

// ----- endpoint drag (onEndpointPreview / onEndpointCommit) -----

// v3 §2b: honor an endpoint-drag snap only onto a REAL, OTHER canonical object (not
// the dragged id, not a transient preview); returns the hover-ring snap or null.
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

// v3 §2b: the ONE undoable batch for an endpoint-drag release — the core's
// endpointReleaseOps (chord-deform edit-geometry + the set-anchor rebind/unbind),
// collapsed to a single ObjectOp. Null when the release is a no-op (core returns []).
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

// ----- create / hover snap canonicalization -----

// W3-G5 (#6): honor a snap ONLY when its target is a real canonical object (the
// transient drag-create preview / snap-indicator ids never are), so a self-snap
// onto the preview is dropped while a real-edge snap is kept.
export function canonicalizeCreateSnap(
  scene: ObjectScene,
  snapped: boolean,
  targetId: string | null
): { snapped: boolean; target: string | null } {
  const target = targetId !== null && scene.objects.some((o) => o.id === targetId) ? targetId : null;
  return { snapped: snapped && target !== null, target };
}

// W3-G9 (#3): the persistent pre-drag hover ring snap — set only when snapped onto a
// real canonical object, else cleared (null). Mirrors create's canonicalization.
export function canonicalizeHoverSnap(
  scene: ObjectScene,
  snapped: boolean,
  targetId: string | null,
  world: { x: number; y: number }
): { at: { x: number; y: number }; target: string } | null {
  const target = targetId !== null && scene.objects.some((o) => o.id === targetId) ? targetId : null;
  return snapped && target !== null ? { at: world, target } : null;
}

// ----- primitive insert / recolor (insertPrimitive / applySelectedColor) -----

// AP1 (#5): the immediate-insert object for `kind` at `center`, painted in
// `selectedColor`, built by the core. The shell authors it as an insert-object op
// and selects it.
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

// AP1 (#5): the set-style op recoloring the single selected object to `color`, or
// null when nothing single is selected. The shell adopts the color regardless (it
// becomes the default for the next NEW shape); only a single selection recolors.
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

// ----- group / double-click (groupSelection / handleObjectDoubleClick) -----

// FC-14: the batch grouping `ids` under a fresh frame — insert a rect frame sized to
// the children's union world-AABB, then reparent each child under it (world-absolute
// transforms reparent unchanged). The frame's id/order come from the shell factories;
// `bounds` is the union AABB (null = degenerate 1x1 at origin).
export function buildGroupOps(
  ids: string[],
  objects: SceneObject[],
  bounds: { minX: number; minY: number; maxX: number; maxY: number } | null,
  frameId: string,
  frameOrder: string,
  rectPath: (w: number, h: number) => string
): { ops: ObjectOp[]; frameId: string } {
  const frame: SceneObject = {
    id: frameId,
    order: frameOrder,
    transform: bounds ? translateTransform(bounds.minX, bounds.minY) : translateTransform(0, 0),
    geometry: {
      d: rectPath(bounds ? bounds.maxX - bounds.minX : 1, bounds ? bounds.maxY - bounds.minY : 1),
      fillRule: "nonZero"
    },
    clip: false
  };
  const ops: ObjectOp[] = [{ kind: "insert-object", object: frame }];
  let order = "a0";
  for (const id of ids) {
    ops.push({ kind: "reparent", id, parent: frame.id, order });
    order = `${order}~`;
  }
  return { ops, frameId: frame.id };
}

// RA2b (#9, D6): the double-click action — the core's container-vs-leaf decision for
// the signal's object. A null signal is a no-op.
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

// ----- context menu assembly (OBJECT_MENU / CANVAS_MENU) -----

// AP3: a context-menu entry — a separator or a command row gated optionally on the
// picked target's shape in the forest (children/parent), beyond the coarse
// `disabledFor` kind check. Icons are decorated by the shell (Svelte components), so
// this layer carries only an opaque icon tag the shell maps; `enabled` is a predicate.
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

// AP3 (#13): ungroup is enabled only for a single container object (has children).
export function ungroupPickEnabled(
  core: Pick<SceneCore, "ungroupEnabled">,
  scene: ObjectScene,
  picked: ObjectSelection
): boolean {
  return picked.kind === "object" && core.ungroupEnabled(scene, picked.id);
}

// AP3 (#18): pop-out is enabled only when the single picked object has a parent.
export function popOutPickEnabled(
  core: Pick<SceneCore, "popOutOp">,
  scene: ObjectScene,
  picked: ObjectSelection
): boolean {
  return picked.kind === "object" && core.popOutOp(scene, picked.id) !== null;
}

// AP3 (#13): the object/multi menu. Icons are tagged by string id and mapped to the
// shell's lucide components at render; `pop-out` is a shell-only command (no catalog
// entry) so it carries its own label.
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

// AP3 (#18, D7): the empty-canvas menu — quick inserts, the template library, and
// select-all. The insert-text entry is gone (D7); text arrives via the toolbar.
export const CANVAS_MENU: ContextMenuEntry<string>[] = [
  { id: "insert-rectangle" },
  { id: "insert-ellipse" },
  { id: "open-template-library", icon: "template" },
  "separator",
  { id: "select-all" }
];

// FC-13: a resolved menu item — the catalog/shell label, the picked-target disabled
// state (kind check OR the `enabled` predicate), plus the icon tag + danger flag.
// `null` is a separator. Pure over the layout + the command catalog + a handler set:
// an entry with no handler drops out (renders nothing).
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

// ----- feed scene + preview cluster (buildFeedScene) -----

// W2-07/Tier-3: the renderer Paint for a transient shell preview's selected color —
// the theme-default sentinel renders as the "text" token, every other color as
// solid. The CANONICAL sentinel→Paint rule lives in the core (build_primitive /
// build_set_style_op); this is the per-frame-preview mirror, kept off the FFI hot path.
export function previewPaint(color: string): Paint {
  return color === THEME_DEFAULT_COLOR ? { kind: "token", name: "text" } : { kind: "solid", color };
}

const q = (px: number) => Math.round(px * GEOMETRY_QUANTUM_PER_PX);

// FC-11: a transient preview object for the in-progress pen stroke — a world-px
// polyline (identity transform, so local==world) with the pen brush. Built in TS
// (NOT op-apply); the committed object replaces it on pointer-up.
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

// W2-07/Tier-3: the transient rubber-band for one drag-create frame. Mirrors the
// core build_primitive_from_drag geometry (line corner-to-corner; closed kinds to
// the normalized bbox) on a pure-translation transform, cheaply in TS so the
// per-frame preview never crosses the FFI boundary.
export function createPreviewObject(
  kind: DragCreateShape,
  span: DragSpan,
  order: string,
  selectedColor: string
): SceneObject {
  let d: string;
  let tx: number;
  let ty: number;
  if (kind === "line") {
    d = `M 0 0 L ${q(span.end.x - span.start.x)} ${q(span.end.y - span.start.y)}`;
    tx = span.start.x;
    ty = span.start.y;
  } else {
    const w = Math.abs(span.end.x - span.start.x);
    const h = Math.abs(span.end.y - span.start.y);
    d = kind === "ellipse" ? previewEllipsePath(w, h) : `M 0 0 L ${q(w)} 0 L ${q(w)} ${q(h)} L 0 ${q(h)} Z`;
    tx = Math.min(span.start.x, span.end.x);
    ty = Math.min(span.start.y, span.end.y);
  }
  return {
    id: "create-preview",
    order,
    transform: translateTransform(tx, ty),
    geometry: { d, fillRule: "nonZero" },
    stroke: { paint: previewPaint(selectedColor), width: 2 * GEOMETRY_QUANTUM_PER_PX, opacity: 1, cap: "butt", join: "miter" }
  };
}

// The four-cubic ellipse rubber-band path (kappa 0.5523), object-local quantized. A
// shell-side preview affordance; the committed ellipse is built by the core.
function previewEllipsePath(w: number, h: number): string {
  const cx = q(w / 2);
  const cy = q(h / 2);
  const kx = Math.round(q(w / 2) * 0.5523);
  const ky = Math.round(q(h / 2) * 0.5523);
  return [
    `M 0 ${cy}`,
    `C 0 ${cy - ky} ${cx - kx} 0 ${cx} 0`,
    `C ${cx + kx} 0 ${q(w)} ${cy - ky} ${q(w)} ${cy}`,
    `C ${q(w)} ${cy + ky} ${cx + kx} ${q(h)} ${cx} ${q(h)}`,
    `C ${cx - kx} ${q(h)} 0 ${cy + ky} 0 ${cy}`,
    "Z"
  ].join(" ");
}

// W2-07: a small ring drawn at a snapped corner so the user sees the snap. Geometry
// is a world-px ellipse (identity transform, so local==world); built in TS like the
// pen preview (NOT op-apply).
export function snapIndicatorObject(at: { x: number; y: number }, order: string): SceneObject {
  const r = 5;
  const k = r * 0.5523;
  const cx = at.x;
  const cy = at.y;
  const d = [
    `M ${q(cx - r)} ${q(cy)}`,
    `C ${q(cx - r)} ${q(cy - k)} ${q(cx - k)} ${q(cy - r)} ${q(cx)} ${q(cy - r)}`,
    `C ${q(cx + k)} ${q(cy - r)} ${q(cx + r)} ${q(cy - k)} ${q(cx + r)} ${q(cy)}`,
    `C ${q(cx + r)} ${q(cy + k)} ${q(cx + k)} ${q(cy + r)} ${q(cx)} ${q(cy + r)}`,
    `C ${q(cx - k)} ${q(cy + r)} ${q(cx - r)} ${q(cy + k)} ${q(cx - r)} ${q(cy)}`,
    "Z"
  ].join(" ");
  return {
    id: "create-snap-indicator",
    order,
    transform: translateTransform(0, 0),
    geometry: { d, fillRule: "nonZero" },
    stroke: { paint: { kind: "solid", color: "#ff3b6b" }, width: 2 * GEOMETRY_QUANTUM_PER_PX, opacity: 1, cap: "round", join: "round" }
  };
}

// FC-11/W2-07: the renderer feed — the canonical scene plus any transient NEW-object
// preview (pen stroke / shape drag-create rubber-band + snap indicator, or the
// persistent pre-drag hover ring when no drag is in progress). Neither mutates the
// canonical scene nor runs op-apply (geometry-vocabulary construction only). The
// order factory positions every preview on top.
export function buildFeedScene(
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
  if (create && createState) {
    const extra: SceneObject[] = [createPreviewObject(create, createState.span, nextOrder(), selectedColor)];
    if (createState.snapped) extra.push(snapIndicatorObject(createState.span.end, nextOrder()));
    feed = { ...feed, objects: [...feed.objects, ...extra] };
  } else if (hoverSnap) {
    // W3-G9 (#3): no drag in progress — render the PERSISTENT pre-drag anchor ring
    // at the hovered edge so the user sees where the next create would anchor.
    feed = { ...feed, objects: [...feed.objects, snapIndicatorObject(hoverSnap.at, nextOrder())] };
  }
  return feed;
}
