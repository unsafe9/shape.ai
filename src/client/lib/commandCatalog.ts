// CC0.2 — TypeScript mirror of the scene-core command catalog
// (crates/scene-core/src/command.rs → command_catalog_json).
//
// The catalog drives two surfaces: the cockpit remote/dispatcher and the
// settings panel's read-only shortcut reference (CC5.1/CC5.2). Keeping a TS
// mirror lets the shell load the catalog without a wasm round-trip; the wasm
// command_catalog_json() is the authoritative source and this list MUST stay in
// sync with it (the cockpit test asserts the id/shortcut set).
//
// CC5.2 binding/command separation: each entry carries a `defaultShortcut`
// (the binding) distinct from `id` (the command). A future binding editor would
// override `defaultShortcut` per command without touching the command set.

export type CommandCategory = "tool" | "shape" | "view" | "edit" | "selection" | "template" | "canvas";

export type Command = {
  id: string;
  label: string;
  category: CommandCategory;
  /** Platform-agnostic binding; "Mod" resolves to Cmd (macOS) / Ctrl elsewhere. Omitted when unbound. */
  defaultShortcut?: string;
  description: string;
};

export const commandCatalog: Command[] = [
  // Tool
  { id: "select-move", label: "Select / Move", category: "tool", defaultShortcut: "V", description: "Activate the select-and-move tool for picking and dragging objects." },
  { id: "hand-pan", label: "Hand / Pan", category: "tool", defaultShortcut: "H", description: "Activate the hand tool to pan the canvas viewport." },
  // Shape
  { id: "insert-rectangle", label: "Rectangle", category: "shape", defaultShortcut: "R", description: "Insert a rectangle shape." },
  { id: "insert-ellipse", label: "Ellipse", category: "shape", defaultShortcut: "O", description: "Insert an ellipse shape." },
  { id: "insert-connector", label: "Connector", category: "shape", defaultShortcut: "C", description: "Insert a connector between two objects." },
  { id: "insert-sticky", label: "Sticky Note", category: "shape", defaultShortcut: "S", description: "Insert a sticky note." },
  { id: "insert-frame", label: "Frame", category: "shape", defaultShortcut: "F", description: "Insert a frame to group objects." },
  // View
  { id: "zoom-in", label: "Zoom In", category: "view", defaultShortcut: "Mod+=", description: "Zoom the canvas in." },
  { id: "zoom-out", label: "Zoom Out", category: "view", defaultShortcut: "Mod+-", description: "Zoom the canvas out." },
  { id: "zoom-fit", label: "Zoom to Fit", category: "view", defaultShortcut: "Shift+1", description: "Zoom and pan so the whole scene fits in the viewport." },
  { id: "zoom-reset", label: "Reset Zoom", category: "view", defaultShortcut: "Mod+0", description: "Reset the zoom level to 100%." },
  { id: "toggle-fullscreen", label: "Toggle Fullscreen", category: "view", defaultShortcut: "F11", description: "Toggle fullscreen canvas mode." },
  // Edit
  { id: "delete", label: "Delete", category: "edit", defaultShortcut: "Backspace", description: "Delete the current selection." },
  { id: "duplicate", label: "Duplicate", category: "edit", defaultShortcut: "Mod+D", description: "Duplicate the current selection." },
  { id: "copy", label: "Copy", category: "edit", defaultShortcut: "Mod+C", description: "Copy the current selection to the clipboard." },
  { id: "paste", label: "Paste", category: "edit", defaultShortcut: "Mod+V", description: "Paste from the clipboard." },
  { id: "group", label: "Group", category: "edit", defaultShortcut: "Mod+G", description: "Group the selected objects into a frame." },
  { id: "ungroup", label: "Ungroup", category: "edit", defaultShortcut: "Mod+Shift+G", description: "Ungroup the selected frame." },
  { id: "align-left", label: "Align Left", category: "edit", description: "Align the selected objects to their left edges." },
  { id: "align-center", label: "Align Center", category: "edit", description: "Align the selected objects to their horizontal centers." },
  { id: "align-right", label: "Align Right", category: "edit", description: "Align the selected objects to their right edges." },
  { id: "distribute-horizontal", label: "Distribute Horizontally", category: "edit", description: "Distribute the selected objects evenly along the horizontal axis." },
  { id: "bring-to-front", label: "Bring to Front", category: "edit", defaultShortcut: "]", description: "Bring the current selection to the front of the z-order." },
  { id: "send-to-back", label: "Send to Back", category: "edit", defaultShortcut: "[", description: "Send the current selection to the back of the z-order." },
  // Selection
  { id: "select-all", label: "Select All", category: "selection", defaultShortcut: "Mod+A", description: "Select all objects in the scene." },
  { id: "clear-selection", label: "Clear Selection", category: "selection", defaultShortcut: "Escape", description: "Clear the current selection." },
  // Template
  { id: "open-template-library", label: "Template Library", category: "template", defaultShortcut: "T", description: "Open the template library." },
  // Canvas
  { id: "new-canvas", label: "New Canvas", category: "canvas", defaultShortcut: "Mod+N", description: "Create a new canvas." },
  { id: "open-settings", label: "Settings", category: "canvas", defaultShortcut: "Mod+,", description: "Open the settings panel." }
];

/**
 * Render a default shortcut for display, resolving the platform-agnostic `Mod`
 * token to the host's primary modifier symbol. Pure; the dispatcher uses
 * {@link parseShortcut} for matching, this is display-only (settings/tooltips).
 */
export function formatShortcut(shortcut: string, isMac = detectMac()): string {
  return shortcut
    .split("+")
    .map((token) => {
      if (token === "Mod") return isMac ? "⌘" : "Ctrl";
      if (token === "Shift") return isMac ? "⇧" : "Shift";
      if (token === "Alt") return isMac ? "⌥" : "Alt";
      return token;
    })
    .join(isMac ? "" : "+");
}

export function detectMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return /mac|iphone|ipad|ipod/i.test(navigator.platform || navigator.userAgent || "");
}
