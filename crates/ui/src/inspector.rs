//! The inspector panel, built PURELY from `inspector_view` (the core decides the
//! role, which controls apply, their values, and `mixed`). For each section this
//! emits a titled vertical container; for each control it maps `control.widget`
//! to a ui-core primitive or a `composites` composite, and reads the display value
//! out of `control.value` (the same paintHex/sizingFixedPx/lanes/align readers the
//! Svelte panel had, moved into Rust). Every interactive part carries an `insp:`
//! id so `intent::resolve` maps its action back to an `InspectorEdit`.

use serde_json::Value;
use shape_scene_core::object::catalog::inspector::{
    InspectorControlValue, InspectorSection, InspectorSectionView, InspectorView, InspectorWidget,
};
use shape_ui_core::{
    Axis, Container, CrossAlign, Edges, MainAlign, Text, TextPaint, Widget, PANEL_RADIUS, ROW_H,
    SPACE_LG, SPACE_MD, SPACE_SM,
};

use crate::composites;

/// The id namespace binding an inspector control to its catalog entry.
pub(crate) const INSPECTOR_PREFIX: &str = "insp:";

const PANEL_W: f64 = 264.0;
const PANEL_MARGIN: f64 = SPACE_LG;
const ROW_GAP: f64 = SPACE_SM;
const SECTION_GAP: f64 = SPACE_MD;
const LABEL_W: f64 = 92.0;
/// Full panel content width — what a stacked control (a segment, with its label above
/// rather than beside it) spans, so its cells are wide enough for their labels.
const SEGMENT_W: f64 = PANEL_W - PANEL_MARGIN * 2.0;
/// The right column a label|control row gives the control — the row's content width
/// minus the label column and the inter-column gap. Derived ONCE here so the inspector
/// and the composites never drift two different control widths apart.
const CONTROL_W: f64 = SEGMENT_W - LABEL_W - SPACE_SM;
/// The bottom band the centered toolbar reserves: its tray height (`PADDING*2 + BTN`
/// = 44) + `BOTTOM_MARGIN` (24) + a breathing gap, so the panel's last section never
/// crosses the tray's top edge. Mirrors `toolbar.rs`'s `ty = vh - tray_h - BOTTOM_MARGIN`.
const TOOLBAR_RESERVE: f64 = 44.0 + 24.0 + 16.0;

/// Build the inspector panel anchored top-right of the viewport. Caller guarantees
/// the view is non-empty (`intent::view_is_empty` gates it). The panel content is one
/// VERTICAL flex stack of section blocks (`SECTION_GAP` between), each itself a
/// vertical flex of `[title, …rows]` (`ROW_GAP` between) — the engine emits every gap,
/// so there is no per-row `cursor_y` to drift.
pub(crate) fn build(view: &InspectorView, viewport: (f64, f64)) -> Widget {
    let (vw, vh) = viewport;
    let x = (vw - PANEL_W - PANEL_MARGIN).max(0.0);

    let blocks: Vec<Widget> = view.sections.iter().map(section_block).collect();
    let content_inner_h = stack_height(&blocks, SECTION_GAP);

    // Grow to fit content, but never past the band above the bottom toolbar. The
    // content top is always pinned at the panel margin so the first section header
    // sits below the rounded corner — there is no clip, so lifting the rows up would
    // spill the top row under the corner (the panel body's radius), the regression
    // this avoids. On overflow the cap bounds the body; the surplus extends past the
    // bottom edge (toward the tray), never above the top inset.
    let content_h = content_inner_h + PANEL_MARGIN * 2.0;
    let max_h = (vh - PANEL_MARGIN - TOOLBAR_RESERVE).max(0.0);
    let panel_h = content_h.min(max_h);

    // The content rides ONE vertical flex inset by the panel margin on every side.
    let content = Widget::Container(Container {
        id: "inspector::content".to_string(),
        x: PANEL_MARGIN,
        y: PANEL_MARGIN,
        w: SEGMENT_W,
        h: content_inner_h,
        direction: Axis::Vertical,
        spacing: SECTION_GAP,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: blocks,
    });

    Widget::Container(Container {
        id: "inspector".to_string(),
        x,
        y: PANEL_MARGIN,
        w: PANEL_W,
        h: panel_h,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: {
            // The macOS-material panel: a soft-shadow underlay + a frosted `material`
            // body with a `hairline` border (radius 14), then the inset content.
            let mut c = Vec::new();
            composites::material_panel(&mut c, "inspector", PANEL_W, panel_h, PANEL_RADIUS);
            c.push(content);
            c
        },
    })
}

/// One section block: its title above its control rows, stacked tight (`ROW_GAP`) in a
/// vertical flex. The block's own height is the laid-out stack height so the parent
/// section-stack can place the next block beneath it.
fn section_block(section: &InspectorSectionView) -> Widget {
    let mut children = vec![section_title(section.section)];
    children.extend(section.controls.iter().map(control_row));
    let h = stack_height(&children, ROW_GAP);
    Widget::Container(Container {
        id: format!("inspector::block::{}", section_key(section.section)),
        x: 0.0,
        y: 0.0,
        w: SEGMENT_W,
        h,
        direction: Axis::Vertical,
        spacing: ROW_GAP,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

/// The laid-out height of a vertical stack: each widget's own height + `gap` between.
/// Mirrors what the vertical flex emits, so a container can size to its content.
fn stack_height(items: &[Widget], gap: f64) -> f64 {
    items.iter().map(widget_height).sum::<f64>() + (items.len().saturating_sub(1)) as f64 * gap
}

fn widget_height(w: &Widget) -> f64 {
    match w {
        Widget::Container(c) => c.h,
        Widget::Rect(r) => r.h,
        Widget::Text(t) => t.h,
        Widget::Button(b) => b.h,
        Widget::Swatch(s) => s.h,
        Widget::Toggle(t) => t.h,
        Widget::Slider(s) => s.h,
        Widget::Segment(s) => s.h,
        Widget::TextInput(t) => t.h,
        Widget::Icon(i) => i.h,
    }
}

fn section_title(section: InspectorSection) -> Widget {
    Widget::Text(Text {
        id: format!("inspector::section::{}", section_key(section)),
        x: 0.0,
        y: 0.0,
        w: SEGMENT_W,
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

/// One control row: a left label Text and the control to its right, laid out
/// HORIZONTALLY with `MainAlign::SpaceBetween` so the label pins to the leading edge
/// and the control to the trailing edge (the engine owns the gap, not a `LABEL_W + 8`
/// cursor). Cross-centered so a 28px control sits on the label's optical row. A button
/// (full-width, no label) and a segment (its own stacked label) keep a single child.
fn control_row(control: &InspectorControlValue) -> Widget {
    let widget = control_widget(control);
    // A button control (canonicalize) spans the full width with no side label; a
    // segment carries its own stacked label (built in `control_widget`).
    let labeled = !matches!(
        control.widget,
        InspectorWidget::Button | InspectorWidget::Segment { .. }
    );
    let children = if labeled {
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
    } else {
        vec![widget]
    };
    // The row is as tall as its tallest child (a stacked segment / align grid runs
    // taller than one row), so the parent section-stack reserves its real extent.
    let h = children.iter().map(widget_height).fold(0.0_f64, f64::max);
    Widget::Container(Container {
        id: format!("inspector::row::{}", control.id),
        x: 0.0,
        y: 0.0,
        w: SEGMENT_W,
        h,
        direction: Axis::Horizontal,
        spacing: 0.0,
        // A labeled row pins label↔control to the two edges; a lone full-width child
        // (button/segment) just sits at the leading edge — `SpaceBetween` is inert for
        // one child, so both cases share the one main-align.
        main_align: MainAlign::SpaceBetween,
        padding: Edges::all(0.0),
        align: CrossAlign::Center,
        children,
    })
}

/// Map a control's widget kind to a ui-core primitive / composite, reading its
/// display value from `control.value`. A `mixed` value renders empty. The control is
/// built at the origin: it is a flex child of `control_row`, which positions it (its
/// own x/y is ignored under `Axis::Horizontal`).
fn control_widget(control: &InspectorControlValue) -> Widget {
    let id = control.id.as_str();
    match &control.widget {
        InspectorWidget::Text => {
            composites::text_input(id, &string_value(control), "", CONTROL_W, ROW_H)
        }
        InspectorWidget::Toggle => composites::toggle_field(id, bool_value(control), 0.0, 0.0),
        InspectorWidget::Badge => {
            composites::badge(id, &string_value(control), 0.0, 0.0, CONTROL_W, 20.0)
        }
        InspectorWidget::Button => {
            composites::action_button(id, &control.label, 0.0, 0.0, SEGMENT_W)
        }
        InspectorWidget::Paint => {
            composites::paint_field(id, &paint_hex(control), 0.0, 0.0, CONTROL_W, ROW_H)
        }
        InspectorWidget::Number { unit, .. } => {
            composites::number_field(id, &number_str(control), unit, 0.0, 0.0, CONTROL_W, ROW_H)
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
                0.0,
                SEGMENT_W,
                ROW_H,
            );
            let body = match sizing_fixed_px(control) {
                Some(px) => stack_fixed_companion(control, seg, px),
                None => seg,
            };
            stack_labeled(&control.id, &control.label, body)
        }
        InspectorWidget::Lanes => composites::lanes_field(
            id,
            lanes_count(control),
            lanes_fill(control),
            0.0,
            0.0,
            CONTROL_W,
            ROW_H,
        ),
        InspectorWidget::Align9 => {
            composites::align9(id, &align_main(control), &align_cross(control), 0.0, 0.0)
        }
    }
}

/// Stack a control's muted label ABOVE its `body`, spanning the full panel content
/// width, as a VERTICAL flex (`ROW_GAP` between). Used for segments, whose cells need
/// the full width — the narrow right-column layout the other controls use would
/// char-wrap a multi-cell segment's labels.
fn stack_labeled(control_id: &str, label: &str, body: Widget) -> Widget {
    let children = vec![
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
    ];
    let h = stack_height(&children, ROW_GAP);
    Widget::Container(Container {
        id: format!("inspector::stacked::{control_id}"),
        x: 0.0,
        y: 0.0,
        w: SEGMENT_W,
        h,
        direction: Axis::Vertical,
        spacing: ROW_GAP,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

/// A sizing segment plus its Fixed-value companion number field, stacked VERTICALLY
/// (`ROW_GAP` between). The companion is right-aligned to the control column width.
fn stack_fixed_companion(control: &InspectorControlValue, seg: Widget, px: f64) -> Widget {
    let companion =
        composites::number_field(&control.id, &fmt_num(px), "px", 0.0, 0.0, CONTROL_W, ROW_H);
    Widget::Container(Container {
        id: format!("{}::sizing", control.id),
        x: 0.0,
        y: 0.0,
        w: SEGMENT_W,
        h: ROW_H * 2.0 + ROW_GAP,
        direction: Axis::Vertical,
        spacing: ROW_GAP,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![seg, companion],
    })
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
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}
