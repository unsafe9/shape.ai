//! The template-library popup: a panel anchored bottom-center above the toolbar
//! listing one row per template. Each row binds to a template id (`template:<id>`),
//! so a press resolves to `Intent::ApplyTemplate` and the shell inserts it — the
//! shell never decides which op a row authors. The template registry is shell/data
//! supplied (each `TemplateEntry` is fed in), like the context-menu item list.

use serde::Deserialize;
use shape_ui_core::{
    Axis, Container, CrossAlign, Edges, MainAlign, Rect, RectStyle, Text, TextPaint, Widget,
};

use crate::composites;
use crate::UiModel;

/// The id namespace binding a template row to a template selection.
pub(crate) const TEMPLATE_PREFIX: &str = "template:";

/// One template-library row the shell feeds: its id (the row binds `template:<id>`),
/// a title, and a one-line description.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
}

const PANEL_W: f64 = 280.0;
const ROW_H: f64 = 44.0;
const PAD: f64 = 8.0;
const BOTTOM_MARGIN: f64 = 76.0;

/// Build the template popup: a panel of rows, bottom-center above the toolbar. The
/// caller (`build_root`) only emits this when `template_open` and the list is
/// non-empty.
pub(crate) fn build(model: &UiModel) -> Widget {
    let rows_n = model.templates.len();
    let panel_h = PAD * 2.0 + rows_n as f64 * ROW_H;
    let (vw, vh) = model.viewport;
    let x = ((vw - PANEL_W) / 2.0).max(0.0);
    let y = (vh - panel_h - BOTTOM_MARGIN).max(0.0);

    // The macOS-material panel: soft-shadow underlay + a frosted `material` body with
    // a `hairline` border (radius 12), then the template rows on top.
    let mut children = Vec::new();
    composites::material_panel(&mut children, "template", PANEL_W, panel_h, 12.0);
    for (i, entry) in model.templates.iter().enumerate() {
        children.push(row(entry, PAD + i as f64 * ROW_H));
    }

    Widget::Container(Container {
        id: "template".to_string(),
        x,
        y,
        w: PANEL_W,
        h: panel_h,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}

/// One actuable row: a `template:<id>` Rect body (the hit target) under a
/// left-aligned title + a muted description line. A Rect body — not a Button — so the
/// title/desc sit left-aligned and stacked instead of the Button's centered label.
fn row(entry: &TemplateEntry, y: f64) -> Widget {
    let inner_w = PANEL_W - PAD * 2.0;
    Widget::Container(Container {
        id: format!("template::row::{}", entry.id),
        x: PAD,
        y,
        w: inner_w,
        h: ROW_H,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children: vec![
            // The hit body is fill-less so the frosted `material` panel shows through;
            // it keeps the `template:<id>` id as the press/resolve target and takes the
            // `hover` background when pointed at (the projection swaps it).
            Widget::Rect(Rect {
                id: format!("{TEMPLATE_PREFIX}{}", entry.id),
                x: 0.0,
                y: 0.0,
                w: inner_w,
                h: ROW_H,
                style: RectStyle {
                    fill: None,
                    stroke: None,
                    corner_radius: 6.0,
                    opacity: 1.0,
                },
                hoverable: true,
            }),
            Widget::Text(Text {
                id: format!("template::title::{}", entry.id),
                x: 8.0,
                y: 6.0,
                w: inner_w - 16.0,
                h: 16.0,
                label: entry.title.clone(),
                size_px: 13.0,
                color: TextPaint::Token("text".to_string()),
                align_center: false,
            }),
            Widget::Text(Text {
                id: format!("template::desc::{}", entry.id),
                x: 8.0,
                y: ROW_H - 18.0,
                w: inner_w - 16.0,
                h: 14.0,
                label: entry.description.clone(),
                size_px: 10.0,
                // The one-line description reads muted (`text-secondary`); the title
                // stays primary `text`.
                color: TextPaint::Token("text-secondary".to_string()),
                align_center: false,
            }),
        ],
    })
}
