//! Domain edits: the ONE authoring vocabulary both the MCP `author` seam and the
//! UI `resolve` seam emit. Each is a pure function that mutates the [`Board`]
//! only — never the scene. The host re-exports the mutated board to ops.
//!
//! Ids are caller-supplied (no rng/time in the pure layer), exactly like the
//! object-MCP create tools; an absent id is an error, not a minted one.

use serde_json::Value;

use crate::model::{Board, Card, Column};

/// The set of domain-mutating tool names. The MCP seam and the UI seam both route
/// a `(tool, args)` pair here.
pub const ADD_COLUMN: &str = "add_column";
pub const ADD_CARD: &str = "add_card";
pub const MOVE_CARD: &str = "move_card";
pub const SET_DONE: &str = "set_done";

/// The one read tool name.
pub const LIST_BOARD: &str = "list_board";

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string arg: {key}"))
}

fn opt_str_arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Apply a named domain edit to `board`. `Err` on an unknown tool or a bad arg —
/// validation lives here so both the AI and the UI hit the same rules.
pub fn apply_edit(board: &mut Board, tool: &str, args: &Value) -> Result<(), String> {
    match tool {
        ADD_COLUMN => add_column(board, args),
        ADD_CARD => add_card(board, args),
        MOVE_CARD => move_card(board, args),
        SET_DONE => set_done(board, args),
        other => Err(format!("unknown kanban author tool: {other}")),
    }
}

/// Append a column. `args: { id, title }`.
fn add_column(board: &mut Board, args: &Value) -> Result<(), String> {
    let id = str_arg(args, "id")?.to_string();
    if board.column(&id).is_some() {
        return Err(format!("column id already exists: {id}"));
    }
    let title = opt_str_arg(args, "title").to_string();
    board.columns.push(Column { id, title, cards: Vec::new() });
    Ok(())
}

/// Append a card to a column. `args: { id, column, title, body? }`.
fn add_card(board: &mut Board, args: &Value) -> Result<(), String> {
    let card_id = str_arg(args, "id")?.to_string();
    if board.find_card(&card_id).is_some() {
        return Err(format!("card id already exists: {card_id}"));
    }
    let column_id = str_arg(args, "column")?;
    let title = opt_str_arg(args, "title").to_string();
    let body = opt_str_arg(args, "body").to_string();
    let column = board
        .column_mut(column_id)
        .ok_or_else(|| format!("no such column: {column_id}"))?;
    column.cards.push(Card { id: card_id, title, body, done: false });
    Ok(())
}

/// Move a card to a target column, appended at the end. `args: { card, toColumn }`.
fn move_card(board: &mut Board, args: &Value) -> Result<(), String> {
    let card_id = str_arg(args, "card")?;
    let to_column = str_arg(args, "toColumn")?.to_string();
    let (ci, ki) = board
        .find_card(card_id)
        .ok_or_else(|| format!("no such card: {card_id}"))?;
    if board.column(&to_column).is_none() {
        return Err(format!("no such column: {to_column}"));
    }
    let card = board.columns[ci].cards.remove(ki);
    // `to_column` validated above, so this lookup always resolves.
    board
        .column_mut(&to_column)
        .expect("target column validated")
        .cards
        .push(card);
    Ok(())
}

/// Flip a card's done flag. `args: { card, done }`.
fn set_done(board: &mut Board, args: &Value) -> Result<(), String> {
    let card_id = str_arg(args, "card")?;
    let done = args
        .get("done")
        .and_then(Value::as_bool)
        .ok_or("missing bool arg: done")?;
    let (ci, ki) = board
        .find_card(card_id)
        .ok_or_else(|| format!("no such card: {card_id}"))?;
    board.columns[ci].cards[ki].done = done;
    Ok(())
}
