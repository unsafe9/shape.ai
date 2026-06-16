//! The domain read projection backing the MCP `list_board` tool: the board as
//! column/card vocabulary (not object ids). Reused as the AI-readable digest of
//! the domain.

use serde_json::{json, Value};

use crate::model::Board;

/// A read projection of the board for the MCP `list_board` tool: column/card
/// vocabulary, not object ids.
pub fn list_board(board: &Board) -> Value {
    json!({
        "columns": board.columns.iter().map(|c| json!({
            "id": c.id,
            "title": c.title,
            "cards": c.cards.iter().map(|k| json!({
                "id": k.id,
                "title": k.title,
                "done": k.done,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}
