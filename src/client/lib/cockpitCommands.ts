// CC1.3 — remote/command → primitive mapping.
//
// The cockpit remote and the shortcut dispatcher both need to turn a command id
// (e.g. "insert-rectangle") into the primitive kind the shell's insertPrimitive
// path understands. Keeping the map here (framework-neutral) lets the cockpit
// test assert the remote insertion mapping without a Svelte mount.

export type PrimitiveKindId = "rectangle" | "ellipse" | "connector" | "sticky" | "frame";

/** The shape commands the cockpit exposes, in remote display order. */
export const insertCommandToPrimitive: Record<string, PrimitiveKindId> = {
  "insert-rectangle": "rectangle",
  "insert-ellipse": "ellipse",
  "insert-connector": "connector",
  "insert-sticky": "sticky",
  "insert-frame": "frame"
};

/** Resolve an insert-* command id to its primitive kind, or null if not a shape command. */
export function primitiveForCommand(commandId: string): PrimitiveKindId | null {
  return insertCommandToPrimitive[commandId] ?? null;
}

export const primitiveOrder: PrimitiveKindId[] = ["rectangle", "ellipse", "connector", "sticky", "frame"];
