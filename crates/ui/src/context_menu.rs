//! The right-click context menu: an absolute panel anchored at the click point with
//! a title + catalog-driven rows. Each row binds to a command-catalog entry by id
//! (`cmd:<command-id>`), so a press resolves to `Intent::Command` exactly like a
//! toolbar button — one binding source. A press on the background scrim dismisses.
//!
//! The shell feeds the resolved item list (which commands apply to the picked
//! selection, plus `enabled`/`danger`, computed shell-side from core queries +
//! its handler map, the same `resolveContextMenuItems` the Svelte shell ran). This
//! layer composes those into widgets; it does not re-derive the layout, keeping
//! `crates/ui` free of shell-coupled gating.

use serde::Deserialize;
use shape_ui_core::{
    Axis, Button, Container, CrossAlign, Edges, MainAlign, Paint, Rect, RectStyle, Text, TextPaint,
    Widget,
};

use crate::composites::{button_style, material_panel};
use crate::toolbar::CMD_PREFIX;
use crate::UiModel;

/// The full-viewport background scrim id — a press dismisses the menu. Distinct from
/// the settings scrim so `intent::resolve` can tell them apart (both map to
/// `Dismiss`, but a separate id keeps the menu's hit box independent).
pub(crate) const SCRIM_ID: &str = "context-menu::scrim";

/// One resolved menu item the shell hands in. `command_id` is the catalog id the row
/// binds to (row id `cmd:<command_id>`), `null` for a separator.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMenuItem {
    /// The catalog command id, or `None` for a separator row.
    #[serde(default)]
    pub command_id: Option<String>,
    pub label: String,
    #[serde(default)]
    pub danger: bool,
    #[serde(default)]
    pub disabled: bool,
}

/// The open context menu: its screen anchor, an optional title (object id / N
/// objects / Canvas), and the resolved item list.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMenuModel {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub title: Option<String>,
    pub items: Vec<ContextMenuItem>,
}

const MENU_W: f64 = 200.0;
const ROW_H: f64 = 30.0;
const TITLE_H: f64 = 26.0;
const SEP_H: f64 = 9.0;
const PAD: f64 = 6.0;

/// Build the menu: a dismiss scrim + an anchored panel. Each non-separator row is a
/// `cmd:<id>` Button (so a press resolves to a Command); a disabled item dims; a
/// danger item is left as a standard button (the row's danger styling rides a token
/// in a later pass, but the binding is what matters here).
pub(crate) fn build(menu: &ContextMenuModel, model: &UiModel) -> Widget {
    let (vw, vh) = model.viewport;

    let mut rows: Vec<Widget> = Vec::new();
    let mut cy = PAD;
    let mut menu_h = PAD * 2.0;
    if let Some(title) = &menu.title {
        rows.push(title_row(title, cy));
        cy += TITLE_H;
        menu_h += TITLE_H;
    }
    for (i, item) in menu.items.iter().enumerate() {
        match &item.command_id {
            None => {
                rows.push(separator(i, cy));
                cy += SEP_H;
                menu_h += SEP_H;
            }
            Some(command_id) => {
                rows.push(item_row(command_id, item, cy));
                cy += ROW_H;
                menu_h += ROW_H;
            }
        }
    }

    // Clamp the anchor so the panel stays on screen.
    let x = menu.x.min((vw - MENU_W).max(0.0)).max(0.0);
    let y = menu.y.min((vh - menu_h).max(0.0)).max(0.0);

    Widget::Container(Container {
        id: "context-menu".to_string(),
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![scrim(vw, vh), panel(x, y, menu_h, rows)],
    })
}

/// A transparent full-viewport scrim that swallows the dismiss press. Painted with a
/// near-zero shadow so it stays click-catching without darkening the canvas (a
/// context menu doesn't dim the page the way the settings modal does).
fn scrim(vw: f64, vh: f64) -> Widget {
    Widget::Rect(Rect {
        id: SCRIM_ID.to_string(),
        x: 0.0,
        y: 0.0,
        w: vw,
        h: vh,
        // A fully-transparent fill: the rect still hit-tests (it owns an id), so a
        // press dismisses, but it does not visually dim the canvas underneath. A
        // `Solid` carries no alpha channel in the renderer — `opacity` is its only
        // alpha lever — so transparency lives in `opacity: 0.0`, not the hex.
        style: RectStyle {
            fill: Some(Paint::Solid("#000000".to_string())),
            stroke: None,
            corner_radius: 0.0,
            opacity: 0.0,
        },
        hoverable: false,
    })
}

fn panel(x: f64, y: f64, h: f64, rows: Vec<Widget>) -> Widget {
    // The macOS-material panel: soft-shadow underlay + a frosted `material` body with a
    // `hairline` border (radius 8, emitting `context-menu::bg`), then the rows on top.
    let mut children = Vec::new();
    material_panel(&mut children, "context-menu", MENU_W, h, 8.0);
    children.extend(rows);
    Widget::Container(Container {
        id: "context-menu::panel".to_string(),
        x,
        y,
        w: MENU_W,
        h,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

fn title_row(title: &str, y: f64) -> Widget {
    Widget::Text(Text {
        id: "context-menu::title".to_string(),
        x: PAD,
        y,
        w: MENU_W - PAD * 2.0,
        h: TITLE_H,
        label: title.to_string(),
        size_px: 11.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    })
}

/// One actuable row bound to a command id (`cmd:<id>`). A disabled item renders with
/// the `Pressed`-dimmed visual and carries no live id (so a press is inert).
fn item_row(command_id: &str, item: &ContextMenuItem, y: f64) -> Widget {
    // A disabled row gets an inert id (no `cmd:` prefix) so a press resolves to
    // nothing; an enabled row carries `cmd:<id>` so it resolves to a Command.
    let id = if item.disabled {
        format!("context-menu::disabled::{command_id}")
    } else {
        format!("{CMD_PREFIX}{command_id}")
    };
    let mut style = button_style(false);
    // A menu item reads as a flat row, not a raised button: drop the border and the
    // resting fill so the frosted `material` panel shows through. Pointing at the row
    // takes the `hover` background — the projection swaps a hovered Button's fill, so
    // the row gives feedback while the press still round-trips off the row id.
    style.stroke = None;
    style.fill = None;
    style.corner_radius = 6.0;
    // A danger item (delete) labels in a literal red; peer/destructive colors are
    // per-action literals, not theme tokens (mirrors the Svelte `danger-menu-item`).
    let label_color = if item.danger {
        TextPaint::Hex("#e5484d".to_string())
    } else {
        TextPaint::Token("text".to_string())
    };
    Widget::Button(Button {
        id,
        x: PAD,
        y,
        w: MENU_W - PAD * 2.0,
        h: ROW_H,
        label: item.label.clone(),
        style,
        label_size_px: 12.0,
        label_color,
    })
}

/// A thin `hairline` divider rect (the macOS separator token).
fn separator(index: usize, y: f64) -> Widget {
    Widget::Rect(Rect {
        id: format!("context-menu::sep::{index}"),
        x: PAD,
        y: y + (SEP_H - 1.0) / 2.0,
        w: MENU_W - PAD * 2.0,
        h: 1.0,
        style: RectStyle {
            fill: Some(Paint::Token("hairline".to_string())),
            stroke: None,
            corner_radius: 0.0,
            opacity: 1.0,
        },
        hoverable: false,
    })
}
