//! The export seam: lower a [`Board`] to scene-core [`ObjectOp`]s so the board
//! RIDES ON the one core canvas. It emits ONLY ops — no second apply.
//!
//! Layout is DERIVED, not stored: each column lowers to a parent group with
//! `Layout { axis: Vertical, lanes: Count{1} }`, and each card to a child rect
//! whose `parent == column.id`. `solve_layout(scene, column.id, deriver)` then
//! packs the cards exactly as the built-in auto-layout does — the extension stores
//! no positions. The board row itself is a horizontal parent group of the column
//! groups. So a layout-shaped domain needs ZERO new layout code.
//!
//! Pure: ids/order come from the injected [`IdOrderAlloc`]; the SAME domain key
//! always yields the SAME object id, so re-export after a domain edit reuses ids
//! (transform-only / zero-rebake), never delete-and-reinsert.

use serde_json::Value;
use shape_extension_contract::{tag_meta, IdOrderAlloc, ObjectMeta, META_MODEL_KEY};
use shape_scene_core::object::{
    semantic_preset_style, Align, AxisSizing, CrossAlign, Fill, FillRule, Geometry, Lanes, Layout,
    LayoutAxis, MainAlign, Object, ObjectOp, PathNode, Sizing, Stroke, SubPath, Text, TextAlign,
    TextRun, TextVAlign, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};

use crate::model::{Board, Card, Column};
use crate::NAME;

/// Card box size (logical px). The column frame is sized to wrap one card width
/// plus the layout spacing inset.
const CARD_W: i32 = 220;
const CARD_H: i32 = 64;
const COL_PAD_PX: i32 = 12;
const COL_W: i32 = CARD_W + 2 * COL_PAD_PX;
/// Tall enough for a starter set; the solver packs within it (a Hug column derives
/// the free frame from this geometry — see layout_solve `pinned_main`).
const COL_H: i32 = 520;
/// The board row group's frame: wide enough for several columns, packed by the
/// solver. Height matches a column.
const BOARD_W: i32 = 1200;
const BOARD_H: i32 = COL_H;
/// Layout spacing (quantized): both the inter-child gap and the container inset.
const SPACING_Q: i32 = COL_PAD_PX * GEOMETRY_QUANTUM_PER_PX;

/// Logical px -> object-local quantized units. Integer multiply stays exact.
const fn px(p: i32) -> i32 {
    p * GEOMETRY_QUANTUM_PER_PX
}

/// A closed rect from (0,0) to (w,h) in object-local quantized units.
fn rect_geometry(w_px: i32, h_px: i32) -> Geometry {
    let (w, h) = (px(w_px), px(h_px));
    Geometry::from_subpaths(
        vec![SubPath {
            closed: true,
            nodes: vec![
                PathNode::corner(0, 0),
                PathNode::corner(w, 0),
                PathNode::corner(w, h),
                PathNode::corner(0, h),
            ],
        }],
        FillRule::EvenOdd,
    )
}

/// A centered label run at the preset size.
fn label(text: &str, color: Option<String>) -> Text {
    Text {
        runs: vec![TextRun {
            text: text.to_string(),
            color,
            size: Some(px(13)),
            bold: false,
            italic: false,
            font: None,
        }],
        align: TextAlign::Center,
        valign: TextVAlign::Middle,
    }
}

/// A top-left heading (column title).
fn heading(text: &str) -> Text {
    Text {
        runs: vec![TextRun {
            text: text.to_string(),
            color: None,
            size: Some(px(14)),
            bold: true,
            italic: false,
            font: None,
        }],
        align: TextAlign::Start,
        valign: TextVAlign::Top,
    }
}

fn meta_for(domain_key: &str) -> Option<ObjectMeta> {
    let mut meta = ObjectMeta::new();
    tag_meta(&mut meta, NAME, domain_key);
    Some(meta)
}

/// The vertical-list layout every column group carries; `Count{1}` = a single
/// track packed in card order.
fn column_layout() -> Layout {
    Layout {
        axis: LayoutAxis::Vertical,
        lanes: Lanes::Count { value: 1 },
        spacing: SPACING_Q,
        align: Align { main: MainAlign::Start, cross: CrossAlign::Start },
    }
}

/// The horizontal-row layout the board group carries (its column groups).
fn board_layout() -> Layout {
    Layout {
        axis: LayoutAxis::Horizontal,
        lanes: Lanes::Count { value: 1 },
        spacing: SPACING_Q,
        align: Align { main: MainAlign::Start, cross: CrossAlign::Start },
    }
}

/// Hug/Hug sizing so the solver derives the container free frame from its own
/// geometry rect (not a Fixed pin), matching layout_solve's geometry-derived path.
fn hug_sizing() -> Sizing {
    Sizing { w: AxisSizing::Hug, h: AxisSizing::Hug }
}

/// Domain keys — the stable `meta[extKey]` per object, the basis for id reuse.
fn root_key() -> String {
    "root".to_string()
}
fn board_row_key() -> String {
    "board".to_string()
}
fn column_key(col: &Column) -> String {
    format!("col:{}", col.id)
}
fn card_key(card: &Card) -> String {
    format!("card:{}", card.id)
}

/// Lower the whole board. Emits, in parents-first order:
/// 1. the root object (carries the model blob + `meta[ext]`),
/// 2. the board-row group (horizontal `Layout`, parented to root),
/// 3. each column group (vertical `Layout`, parented to the board row),
/// 4. each card rect (parented to its column), styled per `done`.
pub fn export_board(
    board: &Board,
    model_json: &Value,
    alloc: &mut IdOrderAlloc,
) -> Result<Vec<ObjectOp>, String> {
    let mut ops = Vec::new();

    let board_row_id = alloc.id(&board_row_key());

    // 1. Root: a CHILDLESS, hidden blob carrier holding the serialized model. It
    //    owns no children (the board row is a top-level object), so the host can
    //    refresh the blob on re-export with a cheap delete+reinsert of just this one
    //    tiny object — no cascade, no rebake of real content. A 1px box keeps it
    //    valid. The model blob lives ONLY here, so the whole extension state rides
    //    one object's meta through op-apply/sync/storage (server stays stateless).
    let root_id = alloc.id(&root_key());
    let mut root = Object::new(root_id, alloc.order(), rect_geometry(1, 1));
    root.transform = Transform3x3::translate(0.0, 0.0);
    root.hidden = true;
    root.name = Some("Kanban board".to_string());
    let mut root_meta = ObjectMeta::new();
    tag_meta(&mut root_meta, NAME, &root_key());
    root_meta.insert(META_MODEL_KEY.to_string(), model_json.clone());
    root.meta = Some(root_meta);
    ops.push(ObjectOp::InsertObject { object: root });

    // 2. Board row: a top-level horizontal parent group of the column groups.
    let mut row = Object::new(board_row_id.clone(), alloc.order(), rect_geometry(BOARD_W, BOARD_H));
    row.layout = Some(board_layout());
    row.sizing = Some(hug_sizing());
    row.fill = Some(Fill {
        paint: shape_scene_core::object::Paint::Token { name: "surface-muted".to_string() },
        opacity: 1.0,
    });
    row.meta = meta_for(&board_row_key());
    ops.push(ObjectOp::InsertObject { object: row });

    // 3 + 4. Columns (parented to the row) then cards (parented to the column).
    for col in &board.columns {
        let col_id = alloc.id(&column_key(col));
        let mut group = Object::new(col_id.clone(), alloc.order(), rect_geometry(COL_W, COL_H));
        group.parent = Some(board_row_id.clone());
        group.layout = Some(column_layout());
        group.sizing = Some(hug_sizing());
        group.fill = Some(Fill {
            paint: shape_scene_core::object::Paint::Token { name: "surface".to_string() },
            opacity: 1.0,
        });
        group.text = Some(heading(&col.title));
        group.name = Some(col.title.clone());
        group.meta = meta_for(&column_key(col));
        ops.push(ObjectOp::InsertObject { object: group });

        for card in &col.cards {
            let card_id = alloc.id(&card_key(card));
            let mut rect = Object::new(card_id, alloc.order(), rect_geometry(CARD_W, CARD_H));
            rect.parent = Some(col_id.clone());
            // Done cards flip to the muted "artifact" preset; open cards use "task".
            let preset = if card.done { "artifact" } else { "task" };
            let (fill, stroke, text_color): (Option<Fill>, Option<Stroke>, Option<String>) =
                semantic_preset_style(preset);
            rect.fill = fill;
            rect.stroke = stroke;
            rect.text = Some(label(&card.title, text_color));
            rect.name = Some(card.title.clone());
            rect.meta = meta_for(&card_key(card));
            ops.push(ObjectOp::InsertObject { object: rect });
        }
    }

    Ok(ops)
}
