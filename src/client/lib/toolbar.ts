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

/** The shape commands the toolbar exposes, in display order. */
export const insertCommandToPrimitive: Record<string, PrimitiveKindId> = {
  "insert-rectangle": "rectangle",
  "insert-ellipse": "ellipse",
  "insert-line": "line",
  "insert-text": "text",
  "insert-frame": "frame"
};

/** Resolve an insert-* command id to its primitive kind, or null if not a shape command. */
export function primitiveForCommand(commandId: string): PrimitiveKindId | null {
  return insertCommandToPrimitive[commandId] ?? null;
}

export const primitiveOrder: PrimitiveKindId[] = ["rectangle", "ellipse", "line", "text", "frame"];
