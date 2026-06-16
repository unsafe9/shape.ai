//! The inspector panel, built PURELY from `inspector_view` (the core decides the
//! role, which controls apply, their values, and `mixed`). For each section this
//! emits a titled vertical container; for each control it maps `control.widget`
//! to a ui-core primitive or a `composites` composite, and reads the display value
//! out of `control.value` (the same paintHex/sizingFixedPx/lanes/align readers the
//! Svelte panel had, moved into Rust). Every interactive part carries an `insp:`
//! id so `intent::resolve` maps its action back to an `InspectorEdit`.

use serde_json::Value;
use shape_scene_core::object::catalog::inspector::{
    InspectorControlValue, InspectorSection, InspectorView, InspectorWidget,
};
use shape_ui_core::{
    Axis, Container, CrossAlign, Edges, Text, TextPaint, Widget,
};

use crate::composites;

/// The id namespace binding an inspector control to its catalog entry.
pub(crate) const INSPECTOR_PREFIX: &str = "insp:";

const PANEL_W: f64 = 264.0;
const PANEL_MARGIN: f64 = 16.0;
const PANEL_RADIUS: f64 = 14.0;
const ROW_H: f64 = 28.0;
const ROW_GAP: f64 = 8.0;
const SECTION_GAP: f64 = 12.0;
const LABEL_W: f64 = 92.0;
const CONTROL_W: f64 = PANEL_W - LABEL_W - 16.0 - 24.0;
/// Full panel content width — what a stacked control (a segment, with its label above
/// rather than beside it) spans, so its cells are wide enough for their labels.
const SEGMENT_W: f64 = PANEL_W - PANEL_MARGIN * 2.0;
/// The bottom band the centered toolbar reserves: its tray height (`PADDING*2 + BTN`
/// = 44) + `BOTTOM_MARGIN` (24) + a breathing gap, so the panel's last section never
/// crosses the tray's top edge. Mirrors `toolbar.rs`'s `ty = vh - tray_h - BOTTOM_MARGIN`.
const TOOLBAR_RESERVE: f64 = 44.0 + 24.0 + 16.0;

/// Build the inspector panel anchored top-right of the viewport. Caller guarantees
/// the view is non-empty (`intent::view_is_empty` gates it).
pub(crate) fn build(view: &InspectorView, viewport: (f64, f64)) -> Widget {
    let (vw, vh) = viewport;
    let x = (vw - PANEL_W - PANEL_MARGIN).max(0.0);

    let mut rows: Vec<Widget> = Vec::new();
    let mut cursor_y = 0.0;
    for section in &view.sections {
        rows.push(section_title(section.section, cursor_y));
        cursor_y += ROW_H;
        for control in &section.controls {
            rows.push(control_row(control, cursor_y));
            cursor_y += control_height(&control.widget) + ROW_GAP;
        }
        cursor_y += SECTION_GAP;
    }

    // Grow to fit content, but never past the band above the bottom toolbar. When the
    // content overflows, cap the panel and scroll the rows up by the overflow so the
    // LAST (Actions) section stays in view above the tray instead of clipping past it.
    let content_h = cursor_y + PANEL_MARGIN * 2.0;
    let max_h = (vh - PANEL_MARGIN - TOOLBAR_RESERVE).max(0.0);
    let panel_h = content_h.min(max_h);
    let scroll_y = (content_h - panel_h).max(0.0);
    Widget::Container(Container {
        id: "inspector".to_string(),
        x,
        y: PANEL_MARGIN,
        w: PANEL_W,
        h: panel_h,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: {
            // The macOS-material panel: a soft-shadow underlay + a frosted `material`
            // body with a `hairline` border (radius 14), then the offset rows on top.
            let mut c = Vec::new();
            composites::material_panel(&mut c, "inspector", PANEL_W, panel_h, PANEL_RADIUS);
            c.extend(rows.into_iter().map(|w| offset(w, PANEL_MARGIN, PANEL_MARGIN - scroll_y)));
            c
        },
    })
}

/// Wrap a widget in an offset absolute container so a row positioned at panel-local
/// (0,y) lands at (dx, y+dy) inside the padded panel.
fn offset(child: Widget, dx: f64, dy: f64) -> Widget {
    Widget::Container(Container {
        id: format!("{}::off", child_id(&child)),
        x: dx,
        y: dy,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![child],
    })
}

fn child_id(w: &Widget) -> String {
    match w {
        Widget::Container(c) => c.id.clone(),
        Widget::Rect(r) => r.id.clone(),
        Widget::Icon(i) => i.id.clone(),
        Widget::Text(t) => t.id.clone(),
        Widget::Button(b) => b.id.clone(),
        Widget::Swatch(s) => s.id.clone(),
        Widget::Toggle(t) => t.id.clone(),
        Widget::Slider(s) => s.id.clone(),
        Widget::Segment(s) => s.id.clone(),
        Widget::TextInput(t) => t.id.clone(),
    }
}

fn section_title(section: InspectorSection, y: f64) -> Widget {
    Widget::Text(Text {
        id: format!("inspector::section::{}", section_key(section)),
        x: 0.0,
        y,
        w: PANEL_W - PANEL_MARGIN * 2.0,
        h: ROW_H,
        label: section_label(section).to_string(),
        size_px: 13.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    })
}

fn section_label(section: InspectorSection) -> &'static str {
    match section {
        InspectorSection::Header => "Object",
        InspectorSection::Placement => "Placement",
        InspectorSection::Layout => "Layout",
        InspectorSection::Appearance => "Appearance",
        InspectorSection::Text => "Text",
        InspectorSection::Action => "Actions",
    }
}

fn section_key(section: InspectorSection) -> &'static str {
    match section {
        InspectorSection::Header => "header",
        InspectorSection::Placement => "placement",
        InspectorSection::Layout => "layout",
        InspectorSection::Appearance => "appearance",
        InspectorSection::Text => "text",
        InspectorSection::Action => "action",
    }
}

/// One control row: a left label Text (except buttons) + the control widget to its
/// right. The control reads its display value from `control.value`.
fn control_row(control: &InspectorControlValue, y: f64) -> Widget {
    let cx = LABEL_W + 8.0;
    let widget = control_widget(control, cx, 0.0);
    // A button control (canonicalize) spans the full width with no side label; a
    // segment carries its own stacked label (built in `control_widget`).
    let children = if matches!(
        control.widget,
        InspectorWidget::Button | InspectorWidget::Segment { .. }
    ) {
        vec![widget]
    } else {
        vec![
            Widget::Text(Text {
                id: format!("inspector::label::{}", control.id),
                x: 0.0,
                y: 0.0,
                w: LABEL_W,
                h: ROW_H,
                label: control.label.clone(),
                size_px: 12.0,
                // Control labels read muted (`text-secondary`); only section titles
                // carry the primary `text` token.
                color: TextPaint::Token("text-secondary".to_string()),
                align_center: false,
            }),
            widget,
        ]
    };
    Widget::Container(Container {
        id: format!("inspector::row::{}", control.id),
        x: 0.0,
        y,
        w: PANEL_W - PANEL_MARGIN * 2.0,
        h: control_height(&control.widget),
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

/// Map a control's widget kind to a ui-core primitive / composite, reading its
/// display value from `control.value`. A `mixed` value renders empty.
fn control_widget(control: &InspectorControlValue, x: f64, y: f64) -> Widget {
    let id = control.id.as_str();
    match &control.widget {
        InspectorWidget::Text => {
            composites::text_input(id, &string_value(control), "", CONTROL_W, ROW_H)
                .pipe(|w| place(w, x, y))
        }
        InspectorWidget::Toggle => composites::toggle_field(id, bool_value(control), x, y),
        InspectorWidget::Badge => {
            composites::badge(id, &string_value(control), x, y, CONTROL_W, 20.0)
        }
        InspectorWidget::Button => composites::action_button(id, &control.label, 0.0, y, PANEL_W - PANEL_MARGIN * 2.0),
        InspectorWidget::Paint => composites::paint_field(id, &paint_hex(control), x, y, CONTROL_W, ROW_H),
        InspectorWidget::Number { unit, .. } => {
            composites::number_field(id, &number_str(control), unit, x, y, CONTROL_W, ROW_H)
        }
        InspectorWidget::Segment { options } => {
            // A segment spans the full panel content width with its label stacked
            // ABOVE it (not in the narrow right column), so every cell is wide enough
            // for its longest label — at the 132px right column a 4-cell text-align or
            // a 2-cell "Horizontal/Vertical" segment char-wrapped ("Cente/r").
            let seg = composites::segment_field(
                id,
                options.clone(),
                segment_selected(control, options),
                0.0,
                ROW_H,
                SEGMENT_W,
                ROW_H,
            );
            let body = match sizing_fixed_px(control) {
                Some(px) => stack_fixed_companion(control, seg, px, 0.0, ROW_H),
                None => seg,
            };
            stack_labeled(&control.id, &control.label, body)
        }
        InspectorWidget::Lanes => {
            composites::lanes_field(id, lanes_count(control), lanes_fill(control), x, y, CONTROL_W, ROW_H)
        }
        InspectorWidget::Align9 => composites::align9(id, &align_main(control), &align_cross(control), x, y),
    }
}

/// Stack a control's muted label ABOVE its `body` (which the caller has already
/// placed at `y = ROW_H`), spanning the full panel content width. Used for segments,
/// whose cells need the full width — the narrow right-column layout the other
/// controls use would char-wrap a multi-cell segment's labels.
fn stack_labeled(control_id: &str, label: &str, body: Widget) -> Widget {
    Widget::Container(Container {
        id: format!("inspector::stacked::{control_id}"),
        x: 0.0,
        y: 0.0,
        w: SEGMENT_W,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![
            Widget::Text(Text {
                id: format!("inspector::label::{control_id}"),
                x: 0.0,
                y: 0.0,
                w: SEGMENT_W,
                h: ROW_H,
                label: label.to_string(),
                size_px: 12.0,
                color: TextPaint::Token("text-secondary".to_string()),
                align_center: false,
            }),
            body,
        ],
    })
}

/// A sizing segment plus its Fixed-value companion number field below it.
fn stack_fixed_companion(
    control: &InspectorControlValue,
    seg: Widget,
    px: f64,
    x: f64,
    y: f64,
) -> Widget {
    let companion = composites::number_field(
        &control.id,
        &fmt_num(px),
        "px",
        x,
        y + ROW_H + ROW_GAP,
        CONTROL_W,
        ROW_H,
    );
    Widget::Container(Container {
        id: format!("{}::sizing", control.id),
        x: 0.0,
        y: 0.0,
        w: PANEL_W,
        h: ROW_H * 2.0 + ROW_GAP,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![seg, companion],
    })
}

/// The pixel height a control occupies (Align9 and a Fixed sizing segment are
/// taller; the rest are one row).
fn control_height(widget: &InspectorWidget) -> f64 {
    match widget {
        InspectorWidget::Align9 => 18.0 * 3.0 + 3.0 * 2.0 + 6.0 + 26.0 + 22.0,
        // A segment stacks its label above the full-width track, so it is two rows tall.
        InspectorWidget::Segment { .. } => ROW_H * 2.0,
        _ => ROW_H,
    }
}

// ---- value readers (ported from InspectorPanel.svelte; null/mixed → empty) ----

fn string_value(control: &InspectorControlValue) -> String {
    match &control.value {
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn bool_value(control: &InspectorControlValue) -> bool {
    matches!(control.value, Value::Bool(true))
}

fn number_str(control: &InspectorControlValue) -> String {
    match control.value.as_f64() {
        Some(n) => fmt_num(n),
        None => String::new(),
    }
}

/// A paint control's current solid hex (the swatch only handles solid paint). A
/// gradient/token/unset/mixed paint reads empty (the swatch shows neutral).
fn paint_hex(control: &InspectorControlValue) -> String {
    if let Value::Object(map) = &control.value {
        if map.get("kind").and_then(Value::as_str) == Some("solid") {
            if let Some(Value::String(color)) = map.get("color") {
                return color.clone();
            }
        }
    }
    String::new()
}

/// A sizing control's current Fixed value in logical px, or `None` when Hug/Fill.
fn sizing_fixed_px(control: &InspectorControlValue) -> Option<f64> {
    let map = control.value.as_object()?;
    if map.get("kind").and_then(Value::as_str) == Some("fixed") {
        return Some(map.get("value").and_then(Value::as_f64).unwrap_or(0.0));
    }
    None
}

fn lanes_fill(control: &InspectorControlValue) -> bool {
    control
        .value
        .as_object()
        .and_then(|m| m.get("kind"))
        .and_then(Value::as_str)
        == Some("fill")
}

fn lanes_count(control: &InspectorControlValue) -> Option<i64> {
    let map = control.value.as_object()?;
    if map.get("kind").and_then(Value::as_str) == Some("count") {
        return Some(map.get("value").and_then(Value::as_i64).unwrap_or(1));
    }
    None
}

fn align_main(control: &InspectorControlValue) -> String {
    control
        .value
        .as_object()
        .and_then(|m| m.get("main"))
        .and_then(Value::as_str)
        .unwrap_or("start")
        .to_string()
}

fn align_cross(control: &InspectorControlValue) -> String {
    control
        .value
        .as_object()
        .and_then(|m| m.get("cross"))
        .and_then(Value::as_str)
        .unwrap_or("start")
        .to_string()
}

/// The selected segment cell index for a control — the inverse of
/// `intent::resolve_segment`'s option→value mapping (the `inspectorSegmentSelected`
/// logic, moved into Rust). A sizing value matches by `kind`; font-weight by the
/// bold boolean; the rest by a lower-cased token. Defaults to cell 0 when
/// unset/mixed (nothing visibly selected drives a benign first-cell highlight, the
/// same as the Svelte panel reading a null value).
fn segment_selected(control: &InspectorControlValue, options: &[String]) -> usize {
    options
        .iter()
        .position(|opt| segment_option_selected(control, opt))
        .unwrap_or(0)
}

fn segment_option_selected(control: &InspectorControlValue, option: &str) -> bool {
    let lower = option.to_lowercase();
    if control.id == "sizing-w" || control.id == "sizing-h" {
        return control
            .value
            .as_object()
            .and_then(|m| m.get("kind"))
            .and_then(Value::as_str)
            == Some(lower.as_str());
    }
    match &control.value {
        Value::Bool(b) => *b == (lower == "bold"),
        Value::String(s) => *s == lower,
        _ => false,
    }
}

/// Format a number for display: round to <=2 decimals, then trim trailing zeros and a
/// trailing dot, so a raw f64 like 310.44776119402985 reads as "310.45" and a whole
/// value as "310" — the field never overflows with full Display precision. The
/// fraction separator is always a PERIOD: Rust's `{:.2}` is locale-independent, but
/// normalizing any `,` to `.` makes the period the field's invariant, not an
/// assumption about the formatter (the inspector showed "226,46" before).
fn fmt_num(n: f64) -> String {
    // `{:.2}` rounds to 2 decimals without a width-narrowing cast.
    let s = format!("{n:.2}").replace(',', ".");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" { "0".to_string() } else { trimmed.to_string() }
}

/// Re-place a widget at a given panel-local origin (text_input is emitted at 0,0).
fn place(widget: Widget, x: f64, y: f64) -> Widget {
    match widget {
        Widget::TextInput(mut t) => {
            t.x = x;
            t.y = y;
            Widget::TextInput(t)
        }
        other => other,
    }
}

/// A tiny pipe helper so the match arms read top-to-bottom.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl Pipe for Widget {}
