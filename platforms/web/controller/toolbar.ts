// Maps a command id (e.g. "insert-rectangle") to the object-primitive kind the shell's insert
// path understands. The geometry templates live in the wasm core; the shell only picks the kind.

export type PrimitiveKindId = "rectangle" | "ellipse" | "line" | "text" | "frame";

// Primitives created by drag-to-create (rubber-band a bbox), vs text/frame which insert at an anchor.
export type DragCreateShape = "rectangle" | "ellipse" | "line";

const DRAG_CREATE_SHAPES: ReadonlySet<PrimitiveKindId> = new Set<PrimitiveKindId>(["rectangle", "ellipse", "line"]);

export function isDragCreateShape(kind: PrimitiveKindId): kind is DragCreateShape {
  return DRAG_CREATE_SHAPES.has(kind);
}

// The insert-* command → primitive kind mapping (shortcuts + context menu). Text/frame stay here
// because they are still authored via shortcut/context-menu; only the toolbar shape buttons drop them.
export const insertCommandToPrimitive: Record<string, PrimitiveKindId> = {
  "insert-rectangle": "rectangle",
  "insert-ellipse": "ellipse",
  "insert-line": "line",
  "insert-text": "text",
  "insert-frame": "frame"
};

// The shape kinds the toolbar shows as buttons, in display order. Text (authored by rect +
// double-click) and frame (not a basic shape) have no button — they survive only via shortcut/context-menu.
export const toolbarShapeKinds: readonly DragCreateShape[] = ["rectangle", "ellipse", "line"];

export function primitiveForCommand(commandId: string): PrimitiveKindId | null {
  return insertCommandToPrimitive[commandId] ?? null;
}

export function toggleColorPopup(open: boolean): boolean {
  return !open;
}

export function toggleStrokePopup(open: boolean): boolean {
  return !open;
}

export const primitiveOrder: PrimitiveKindId[] = ["rectangle", "ellipse", "line", "text", "frame"];
