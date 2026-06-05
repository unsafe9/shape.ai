/**
 * T4.2 — Todo / Task Board Template
 *
 * A concrete TemplateContract instance (authored against the T4.1 contract)
 * representing a Kanban-style todo/task board with:
 *   - Frames as board columns (To Do / In Progress / Done)
 *   - Cards as task items with status/owner/priority carried in meta
 *   - Status, owner, and priority as meta + status-color Tag chips (T2.4)
 *   - Dependency edges between tasks
 *   - Comments via the promoted add-comment op (T2.5)
 *
 * The template lowers entirely to create-group / create-card / create-edge ops,
 * so the produced objects are ordinary SceneGroup / SceneNode / SceneEdge rows
 * distinguishable solely by meta.templateKind="todo".
 *
 * Both human (canvas tools) and MCP (patch_scene / add_comment) edits funnel
 * through the same applyRenderPatchToShapeScene → commitAppPatch path.
 */

import type { TemplateContract } from "./contract";
import { z } from "zod";

// ---------------------------------------------------------------------------
// Per-template meta Zod schema (T4.1 Open question / T4.2 §Open questions)
// Validated at authoring/apply time only — never gates renderer or core.
// ---------------------------------------------------------------------------

export const todoBoardNodeMetaSchema = z.object({
  templateKind: z.literal("todo"),
  semanticType: z.literal("task"),
  status: z.enum(["todo", "doing", "done", "blocked"]).default("todo"),
  owner: z.string().default(""),
  priority: z.enum(["low", "normal", "high"]).default("normal")
});

export const todoBoardFrameMetaSchema = z.object({
  templateKind: z.literal("todo"),
  semanticType: z.enum(["board", "column"]),
  column: z.enum(["todo", "doing", "done"]).optional()
});

export type TodoBoardNodeMeta = z.infer<typeof todoBoardNodeMetaSchema>;
export type TodoBoardFrameMeta = z.infer<typeof todoBoardFrameMetaSchema>;

// ---------------------------------------------------------------------------
// Template contract instance
// ---------------------------------------------------------------------------

/**
 * The todo/task-board TemplateContract.
 *
 * Recipe structure:
 *   Frames:  f-board (root) → f-todo, f-doing, f-done (child columns)
 *   Shapes:  t-1, t-2, t-3 (one task per column as default seed)
 *   Edges:   e-1 (t-2 depends_on t-1), dependency edge
 *
 * Column meta uses semanticType:"column" + column:<slug> so the shell can
 * render column headers without a special object type. Board membership
 * (which column a card is in) is SceneNode.groupId — changed by set-object-group.
 *
 * Tag local ids map to suggested status/priority chips seeded in tags.suggested.
 */
export const todoBoardTemplate: TemplateContract = {
  // §1 Metadata
  metadata: {
    id: "todo-board",
    title: "Todo / Task Board",
    description:
      "Columns of task cards with status, owner, priority, and dependency edges.",
    category: "planning",
    icon: "board",
    templateKind: "todo"
  },

  // §2 Primitive recipe
  recipe: {
    frames: [
      // Root board frame
      {
        localId: "f-board",
        title: "Task Board",
        meta: { templateKind: "todo", semanticType: "board" }
      },
      // Column frames (children of f-board)
      {
        localId: "f-todo",
        title: "To Do",
        parentLocalId: "f-board",
        meta: { templateKind: "todo", semanticType: "column", column: "todo" }
      },
      {
        localId: "f-doing",
        title: "In Progress",
        parentLocalId: "f-board",
        meta: { templateKind: "todo", semanticType: "column", column: "doing" }
      },
      {
        localId: "f-done",
        title: "Done",
        parentLocalId: "f-board",
        meta: { templateKind: "todo", semanticType: "column", column: "done" }
      }
    ],

    shapes: [
      // Seed task in "To Do" column
      {
        localId: "t-1",
        frameLocalId: "f-todo",
        title: "Define requirements",
        summary: "Capture the acceptance criteria for the feature.",
        detail: "",
        styleKey: "task",
        tagLocalIds: ["tg-status-todo", "tg-priority-normal"],
        meta: {
          templateKind: "todo",
          semanticType: "task",
          status: "todo",
          owner: "",
          priority: "normal"
        },
        position: { x: 0, y: 0 },
        size: { width: 390, height: 160 }
      },
      // Seed task in "In Progress" column
      {
        localId: "t-2",
        frameLocalId: "f-doing",
        title: "Wire up auth",
        summary: "Add OAuth callback route and session handling.",
        detail: "",
        styleKey: "task",
        tagLocalIds: ["tg-status-doing", "tg-priority-high"],
        meta: {
          templateKind: "todo",
          semanticType: "task",
          status: "doing",
          owner: "",
          priority: "high"
        },
        position: { x: 470, y: 0 },
        size: { width: 390, height: 160 }
      },
      // Seed task in "Done" column
      {
        localId: "t-3",
        frameLocalId: "f-done",
        title: "Scaffold project",
        summary: "Initialize repo, CI, and base configuration.",
        detail: "",
        styleKey: "task",
        tagLocalIds: ["tg-status-done"],
        meta: {
          templateKind: "todo",
          semanticType: "task",
          status: "done",
          owner: "",
          priority: "normal"
        },
        position: { x: 940, y: 0 },
        size: { width: 390, height: 160 }
      }
    ],

    edges: [
      // t-2 depends on t-1 (t-1 must be done before t-2 can proceed)
      {
        localId: "e-1",
        frameLocalId: "f-board",
        sourceLocalId: "t-2",
        targetLocalId: "t-1",
        label: "blocks",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      }
    ]
  },

  // §3 Default layout
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 390, height: 160 }
  },

  // §4 Allowed exports — ai_plan_md reads type:"task" nodes; mermaid renders dep edges
  exports: {
    allowed: ["ai_plan_md", "mermaid"],
    default: "ai_plan_md"
  },

  // §5 Suggested tags — status chips + priority chips seeded into the Tag registry
  tags: {
    suggested: [
      // Status chips (mirror T2.4 status-color convention)
      { localId: "tg-status-todo", name: "To Do", color: "#94a3b8" },
      { localId: "tg-status-doing", name: "In Progress", color: "#3b82f6" },
      { localId: "tg-status-done", name: "Done", color: "#22c55e" },
      { localId: "tg-status-blocked", name: "Blocked", color: "#ef4444" },
      // Priority chips
      { localId: "tg-priority-low", name: "Low", color: "#a3e635" },
      { localId: "tg-priority-normal", name: "Normal", color: "#facc15" },
      { localId: "tg-priority-high", name: "High", color: "#f97316" }
    ]
  },

  // §6 Optional AI prompt hints (advisory; never gating renderer)
  promptHints: {
    systemHint:
      "This is a todo/task board. Cards are tasks; frames are Kanban columns. " +
      "Move a card between columns with set-object-group. " +
      "Add a dependency with create-edge (styleKey:depends_on). " +
      "Change status/owner/priority by updating meta on the card.",
    fieldHints: {
      task:
        "meta.status ∈ {todo,doing,done,blocked}; meta.priority ∈ {low,normal,high}; " +
        "meta.owner is a free-form handle or name.",
      column:
        "meta.column ∈ {todo,doing,done} identifies the column's semantic role. " +
        "Add/rename/remove columns with create-group/edit-text/delete-group."
    },
    suggestedOperations: [
      "create-card",
      "edit-card-text",
      "move-card",
      "set-object-group",
      "create-edge",
      "delete-edge",
      "set-object-tags",
      "add-comment",
      "delete-card"
    ]
  }
};
