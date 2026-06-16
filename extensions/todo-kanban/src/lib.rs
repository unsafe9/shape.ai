//! `todo-kanban`: a reference data-rep extension proving a LAYOUT-SHAPED domain
//! (list/board) maps onto the core Flow solver with zero new layout code.
//!
//! Domain model `Board { columns[cards] }` is the source of truth; columns lower
//! to parent groups with `Layout`, cards to child rects, so `solve_layout` derives
//! the packing (see `export`). The seams: MCP (`edits` + the `read` projection +
//! the `Extension` impl), export (`export`). All pure: no time/rng/IO.

mod edits;
mod export;
mod model;
mod read;

use serde_json::Value;
use shape_extension_contract::{Extension, IdOrderAlloc, McpToolMeta};
use shape_scene_core::object::ObjectOp;

use crate::edits::{ADD_CARD, ADD_COLUMN, LIST_BOARD, MOVE_CARD, SET_DONE};
use crate::model::Board;

/// The extension name: the `ext_kanban_<tool>` MCP namespace and the `meta[ext]`
/// tag value.
pub const NAME: &str = "kanban";

/// The zero-sized extension handle the host boxes into its registry.
#[derive(Clone, Copy, Debug, Default)]
pub struct KanbanExtension;

/// Deserialize the model JSON into a [`Board`] (an empty/absent value => empty
/// board), so a fresh root or a malformed blob never panics the host.
fn board_of(model: &Value) -> Result<Board, String> {
    if model.is_null() {
        return Ok(Board::default());
    }
    serde_json::from_value(model.clone()).map_err(|e| format!("invalid kanban model: {e}"))
}

fn board_to_json(board: &Board) -> Value {
    serde_json::to_value(board).expect("Board serializes")
}

impl Extension for KanbanExtension {
    fn name(&self) -> &'static str {
        NAME
    }

    fn mcp_tools(&self) -> Vec<McpToolMeta> {
        use serde_json::json;
        vec![
            McpToolMeta {
                name: LIST_BOARD,
                description: "List the kanban board as columns and their cards (domain vocabulary).",
                write: false,
                schema: json!({ "type": "object", "properties": {} }),
            },
            McpToolMeta {
                name: ADD_COLUMN,
                description: "Append a column to the board.",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": { "id": { "type": "string" }, "title": { "type": "string" } },
                    "required": ["id"],
                }),
            },
            McpToolMeta {
                name: ADD_CARD,
                description: "Append a card to a column.",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "column": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                    },
                    "required": ["id", "column"],
                }),
            },
            McpToolMeta {
                name: MOVE_CARD,
                description: "Move a card to another column (appended at the end).",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": { "card": { "type": "string" }, "toColumn": { "type": "string" } },
                    "required": ["card", "toColumn"],
                }),
            },
            McpToolMeta {
                name: SET_DONE,
                description: "Set a card's done flag (flips its semantic style on export).",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": { "card": { "type": "string" }, "done": { "type": "boolean" } },
                    "required": ["card", "done"],
                }),
            },
        ]
    }

    fn read(&self, model: &Value, tool: &str, _args: &Value) -> Result<Value, String> {
        let board = board_of(model)?;
        match tool {
            LIST_BOARD => Ok(read::list_board(&board)),
            other => Err(format!("unknown kanban read tool: {other}")),
        }
    }

    fn author(&self, model: &Value, tool: &str, args: &Value) -> Result<Value, String> {
        let mut board = board_of(model)?;
        edits::apply_edit(&mut board, tool, args)?;
        Ok(board_to_json(&board))
    }

    fn export(&self, model: &Value, alloc: &mut IdOrderAlloc) -> Result<Vec<ObjectOp>, String> {
        let board = board_of(model)?;
        export::export_board(&board, model, alloc)
    }

    fn empty_model(&self) -> Value {
        board_to_json(&Board::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use shape_extension_contract::{META_EXT_KEY, META_MODEL_KEY};
    use shape_scene_core::object::{
        apply_object_op, solve_layout, ObjectScene, StubOutlineDeriver,
    };

    /// Drive the REAL core op-apply (no second apply, ever) over a Vec<ObjectOp>.
    fn apply_all(ops: Vec<ObjectOp>) -> ObjectScene {
        let mut scene = ObjectScene::default();
        for op in ops {
            apply_object_op(&mut scene, op).expect("op applies through the real core");
        }
        scene
    }

    /// A board: one column "To do" with two cards, authored through the MCP author
    /// seam so the test exercises the real authoring path.
    fn authored_board() -> Value {
        let ext = KanbanExtension;
        let mut model = ext.empty_model();
        model = ext
            .author(&model, ADD_COLUMN, &json!({ "id": "todo", "title": "To do" }))
            .unwrap();
        model = ext
            .author(&model, ADD_CARD, &json!({ "id": "k1", "column": "todo", "title": "First" }))
            .unwrap();
        model = ext
            .author(&model, ADD_CARD, &json!({ "id": "k2", "column": "todo", "title": "Second" }))
            .unwrap();
        model
    }

    // (a) export() round-trip: author -> export -> drive the REAL apply -> assert
    // solve_layout over the exported column group returns N child placements in
    // card order. This is THE proof the layout-shaped domain rides the core solver.
    #[test]
    fn solve_layout_packs_exported_cards_in_card_order() {
        let ext = KanbanExtension;
        let model = authored_board();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);

        let col_id = alloc.id("col:todo");
        let placements = solve_layout(&scene, &col_id, &StubOutlineDeriver);
        assert_eq!(placements.len(), 2, "two cards packed by the core solver");

        let card1 = alloc.id("card:k1");
        let card2 = alloc.id("card:k2");
        assert_eq!(placements[0].0, card1, "first card first (card order)");
        assert_eq!(placements[1].0, card2, "second card second");

        // Vertical list: card 2 sits strictly below card 1 (the solver derived it).
        let y1 = placements[0].1.m[1][2];
        let y2 = placements[1].1.m[1][2];
        assert!(y2 > y1, "second card packed below the first: {y1} -> {y2}");
    }

    /// The board row group packs its column groups horizontally (the solver, again).
    #[test]
    fn solve_layout_packs_columns_horizontally_in_the_board_row() {
        let ext = KanbanExtension;
        let mut model = authored_board();
        model = ext
            .author(&model, ADD_COLUMN, &json!({ "id": "doing", "title": "Doing" }))
            .unwrap();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);

        let row_id = alloc.id("board");
        let cols = solve_layout(&scene, &row_id, &StubOutlineDeriver);
        assert_eq!(cols.len(), 2, "two column groups packed in the row");
        let x0 = cols[0].1.m[0][2];
        let x1 = cols[1].1.m[0][2];
        assert!(x1 > x0, "second column packed to the right: {x0} -> {x1}");
    }

    /// Every exported object carries `meta[ext] = "kanban"`, and the root carries
    /// the model blob — so a round-trip / incremental re-export finds the objects.
    #[test]
    fn exported_objects_are_tagged_and_root_carries_the_model() {
        let ext = KanbanExtension;
        let model = authored_board();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);

        for o in &scene.objects {
            let meta = o.meta.as_ref().expect("every exported object has meta");
            assert_eq!(meta[META_EXT_KEY], json!("kanban"), "object {} tagged", o.id);
        }
        let root = scene.get(&alloc.id("root")).expect("root exists");
        let blob = &root.meta.as_ref().unwrap()[META_MODEL_KEY];
        // The blob round-trips back to the same board.
        let restored: Board = serde_json::from_value(blob.clone()).unwrap();
        assert_eq!(restored.columns.len(), 1);
        assert_eq!(restored.columns[0].cards.len(), 2);
    }

    // (b) export determinism with a fixed allocator: byte-identical ops.
    #[test]
    fn export_is_deterministic_with_a_fixed_allocator() {
        let ext = KanbanExtension;
        let model = authored_board();
        let ops_a = ext.export(&model, &mut IdOrderAlloc::new(NAME)).unwrap();
        let ops_b = ext.export(&model, &mut IdOrderAlloc::new(NAME)).unwrap();
        assert_eq!(ops_a, ops_b, "same model + fixed allocator => byte-identical ops");
    }

    /// Re-export after a domain edit REUSES ids (keyed off the stable domain key),
    /// so the card objects keep their ids — the incremental / zero-rebake contract.
    #[test]
    fn reexport_reuses_card_ids_for_unchanged_cards() {
        let ext = KanbanExtension;
        let model = authored_board();
        let id_before = IdOrderAlloc::new(NAME).id("card:k1");
        // A done edit changes only card k1's style, not its id.
        let edited = ext
            .author(&model, SET_DONE, &json!({ "card": "k1", "done": true }))
            .unwrap();
        let id_after = IdOrderAlloc::new(NAME).id("card:k1");
        assert_eq!(id_before, id_after, "card id stable across a domain edit");

        // The re-export still contains the same card object id (reuse, not reinsert).
        let ops = ext.export(&edited, &mut IdOrderAlloc::new(NAME)).unwrap();
        let scene = apply_all(ops);
        assert!(scene.get(&id_after).is_some(), "k1 object id reused after edit");
    }

    /// `set_done` has a falsifiable EFFECT: authoring it flips the model flag AND
    /// the re-exported card rect adopts the muted "artifact" preset (vs the "task"
    /// preset an open card carries). FAILS if `edits::set_done` ignores `done`.
    #[test]
    fn set_done_flips_the_model_flag_and_the_exported_card_style() {
        use shape_scene_core::object::semantic_preset_style;
        let ext = KanbanExtension;
        let model = authored_board(); // k1 starts open (not done)
        let (open_fill, _, _) = semantic_preset_style("task");
        let (done_fill, _, _) = semantic_preset_style("artifact");
        assert_ne!(open_fill, done_fill, "the two presets differ, so the flip is observable");

        let card_id = IdOrderAlloc::new(NAME).id("card:k1");
        let fill_of = |m: &Value| {
            let scene = apply_all(ext.export(m, &mut IdOrderAlloc::new(NAME)).unwrap());
            scene.get(&card_id).unwrap().fill.clone()
        };
        assert_eq!(fill_of(&model), open_fill, "open card uses the task preset");

        let done = ext.author(&model, SET_DONE, &json!({ "card": "k1", "done": true })).unwrap();
        assert_eq!(done["columns"][0]["cards"][0]["done"], json!(true), "model flag set");
        assert_eq!(fill_of(&done), done_fill, "done card flips to the artifact preset");
    }

    #[test]
    fn read_list_board_projects_domain_vocabulary() {
        let ext = KanbanExtension;
        let model = authored_board();
        let listed = ext.read(&model, LIST_BOARD, &json!({})).unwrap();
        assert_eq!(listed["columns"][0]["id"], json!("todo"));
        assert_eq!(listed["columns"][0]["cards"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn author_rejects_unknown_tool_and_bad_args() {
        let ext = KanbanExtension;
        let model = ext.empty_model();
        assert!(ext.author(&model, "nope", &json!({})).is_err());
        assert!(ext.author(&model, ADD_CARD, &json!({ "id": "x" })).is_err(), "missing column");
    }
}
