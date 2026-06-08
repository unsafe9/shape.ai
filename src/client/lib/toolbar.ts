// U1 — toolbar object-primitive mapping (renamed from cockpitCommands).
//
// The toolbar and the shortcut dispatcher both turn a command id (e.g.
// "insert-rectangle") into the object-primitive kind the shell's insert path
// understands. Keeping the map here (framework-neutral) lets the toolbar test
// assert the insertion mapping without a Svelte mount. Object primitives are
// closed/open path-string geometries the shell lowers to an `insert-object`
// ObjectOp (the geometry templates live in the wasm core; the shell only picks
// the kind).

export type PrimitiveKindId = "rectangle" | "ellipse" | "line" | "text" | "frame";

// W2-07: the primitives created by drag-to-create (rubber-band a bbox), as opposed
// to text/frame which still insert immediately at an anchor.
export type DragCreateShape = "rectangle" | "ellipse" | "line";

const DRAG_CREATE_SHAPES: ReadonlySet<PrimitiveKindId> = new Set<PrimitiveKindId>(["rectangle", "ellipse", "line"]);

/** True when a primitive kind is a drag-to-create shape (W2-07). */
export function isDragCreateShape(kind: PrimitiveKindId): kind is DragCreateShape {
  return DRAG_CREATE_SHAPES.has(kind);
}

/** The insert-* command → primitive kind mapping (shortcuts + context menu).
 * Text/frame stay here because they are still authored via shortcut/context-menu;
 * only the toolbar's shape *buttons* (see `toolbarShapeKinds`) drop them (D7). */
export const insertCommandToPrimitive: Record<string, PrimitiveKindId> = {
  "insert-rectangle": "rectangle",
  "insert-ellipse": "ellipse",
  "insert-line": "line",
  "insert-text": "text",
  "insert-frame": "frame"
};

/** The shape kinds the toolbar shows as buttons, in display order. D7: text is
 * authored by rect + double-click and frame is not a basic shape, so neither has
 * a toolbar button — they survive only via shortcut/context-menu. */
export const toolbarShapeKinds: readonly DragCreateShape[] = ["rectangle", "ellipse", "line"];

/** Resolve an insert-* command id to its primitive kind, or null if not a shape command. */
export function primitiveForCommand(commandId: string): PrimitiveKindId | null {
  return insertCommandToPrimitive[commandId] ?? null;
}

/**
 * TB1 (#3): the color control is a single rainbow swatch that TOGGLES a popup —
 * clicking it while closed opens, while open closes. Pure so the toolbar test can
 * pin the open->close-on-re-click behavior without a Svelte mount.
 */
export function toggleColorPopup(open: boolean): boolean {
  return !open;
}

export const primitiveOrder: PrimitiveKindId[] = ["rectangle", "ellipse", "line", "text", "frame"];
