//! The kanban DOMAIN model — the extension's own typed data, NOT scene-core types.
//! It is the source of truth for the extension; scene-core objects are its rendered
//! + collaborative projection (see `export`). The board layout is NOT stored here:
//! card placement is DERIVED by the core Flow solver over the exported groups.

use serde::{Deserialize, Serialize};

/// A board = an ordered row of columns; each column an ordered stack of cards.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub title: String,
    pub cards: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub done: bool,
}

impl Board {
    pub fn column(&self, id: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.id == id)
    }

    pub fn column_mut(&mut self, id: &str) -> Option<&mut Column> {
        self.columns.iter_mut().find(|c| c.id == id)
    }

    /// `(column index, card index)` of `card_id`, searching every column.
    pub fn find_card(&self, card_id: &str) -> Option<(usize, usize)> {
        for (ci, col) in self.columns.iter().enumerate() {
            if let Some(ki) = col.cards.iter().position(|k| k.id == card_id) {
                return Some((ci, ki));
            }
        }
        None
    }
}
