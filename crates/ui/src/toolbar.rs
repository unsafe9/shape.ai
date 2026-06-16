//! The bottom-center toolbar: ONE rounded `material` tray (a soft-shadow underlay
//! under a translucent frosted body) holding icon-only command buttons, each bound
//! to an `object_command_catalog` entry by id (`cmd:<command-id>`). Tool buttons
//! (`select-move`/`hand-pan`/`draw`/`erase`) and insert buttons mark active off the
//! model's `active_tool`/`create_kind` mirror — a VisualState, never a raw flag
//! branch. Thin `hairline` separators divide the catalog groups; the inline
//! pen-width chips + color swatches trail the command row. Verifying a button = the
//! command appears in settings (Cmd+,), the single-source rule.

use shape_scene_core::object::catalog::commands::{ObjectCommand, ObjectCommandCategory};
use shape_ui_core::{
    Axis, Button, Container, CrossAlign, Edges, Paint, Rect, RectStyle, Swatch, TextPaint, Widget,
};

use crate::composites::{button_style, soft_shadow};
use crate::icons::icon_in_box;
use crate::UiModel;

/// The id namespace binding a toolbar/context button to a command-catalog entry.
pub(crate) const CMD_PREFIX: &str = "cmd:";
/// The id namespace binding a color chip to a pen-color selection.
pub(crate) const SWATCH_PREFIX: &str = "swatch:";
/// The id namespace binding a brush-size chip to a pen-width selection.
pub(crate) const PEN_WIDTH_PREFIX: &str = "pen-width:";

/// The command ids the toolbar surfaces, in display order. Each MUST exist in
/// `object_command_catalog` (the `toolbar_ids_are_all_in_the_command_catalog` test
/// pins this) — the toolbar holds no command not in the single source. The blank
/// markers split the catalog GROUPS the tray draws a `hairline` separator between
/// (tool | insert | edit | history | more | view).
pub(crate) const TOOLBAR_GROUPS: &[&[&str]] = &[
    // Tool group.
    &["select-move", "hand-pan", "draw", "erase"],
    // Insert group.
    &["insert-rectangle", "insert-ellipse", "insert-line", "insert-text", "insert-frame"],
    // Edit group.
    &["duplicate", "delete", "group", "ungroup"],
    // History group.
    &["undo", "redo"],
    // More group: template library + export + diagnostics toggle.
    &["open-template-library", "export", "toggle-diagnostics"],
    // View / zoom group.
    &["zoom-out", "zoom-in", "zoom-fit", "toggle-fullscreen"],
];

const BTN: f64 = 32.0;
const SWATCH_W: f64 = 24.0;
const CHIP_W: f64 = 34.0;
const SPACING: f64 = 4.0;
/// The gap a group separator occupies (the hairline is centered in it).
const SEP_GAP: f64 = 9.0;
const SEP_W: f64 = 1.0;
const PADDING: f64 = 6.0;
const TRAY_RADIUS: f64 = 14.0;
const BTN_RADIUS: f64 = 8.0;
const BOTTOM_MARGIN: f64 = 24.0;

/// Build the bottom-center toolbar: ONE rounded `material` tray (soft-shadow
/// underlay → frosted body → `hairline` group separators → icon buttons → inline
/// pen-width chips + color swatches). Each command is a `cmd:<id>` icon Button (the
/// matching tool/insert marked active off the `active_tool`/`create_kind` mirror);
/// each width chip is a `pen-width:<px>` Button and each color chip a `swatch:<hex>`
/// Swatch, marked selected off the `pen_width`/`selected_color` mirror. The
/// inline palette/width rows are the Color/Stroke controls Toolbar.svelte carried;
/// the OS custom-color picker stays a shell host surface (like IME).
pub(crate) fn build(model: &UiModel) -> Widget {
    // First pass: total inner width = command groups (BTN each + separators between
    // groups) + the trailing pen-width chips + color swatches.
    let group_extents: Vec<f64> = TOOLBAR_GROUPS
        .iter()
        .map(|g| {
            let n = g.iter().filter(|id| command(model.command_catalog, id).is_some()).count();
            row_extent(n, BTN)
        })
        .collect();
    let cmd_w: f64 = group_extents.iter().sum::<f64>()
        + (group_extents.len().saturating_sub(1)) as f64 * SEP_GAP;

    let trailing = model.pen_widths.len() + model.pen_palette.len();
    // A separator divides the command row from the inline chips when either is shown.
    let trail_sep = if trailing > 0 && cmd_w > 0.0 { SEP_GAP } else { 0.0 };
    let chips_w = row_extent(model.pen_widths.len(), CHIP_W);
    let swatches_w = row_extent(model.pen_palette.len(), SWATCH_W);
    let chip_swatch_join = if !model.pen_widths.is_empty() && !model.pen_palette.is_empty() {
        SPACING
    } else {
        0.0
    };
    let inner_w = cmd_w + trail_sep + chips_w + chip_swatch_join + swatches_w;

    let tray_w = PADDING * 2.0 + inner_w;
    let tray_h = PADDING * 2.0 + BTN;
    let (vw, vh) = model.viewport;
    let tx = ((vw - tray_w) / 2.0).max(0.0);
    let ty = (vh - tray_h - BOTTOM_MARGIN).max(0.0);

    // Children placed ABSOLUTELY (Axis::None) so paint/hit order is explicit: the
    // shadow + frosted body + separators paint first (behind), then the buttons /
    // chips / swatches (last in tree order → they win the hit and paint on top).
    let mut children: Vec<Widget> = Vec::new();
    soft_shadow(&mut children, "toolbar", tray_w, tray_h, TRAY_RADIUS);
    children.push(tray_body(tray_w, tray_h));

    // Second pass: place each group's buttons, a separator between groups.
    let mut cx = PADDING;
    let cy = PADDING;
    for (g, group) in TOOLBAR_GROUPS.iter().enumerate() {
        if g > 0 {
            push_separator(&mut children, cx, tray_h);
            cx += SEP_GAP;
        }
        for id in group.iter() {
            let Some(cmd) = command(model.command_catalog, id) else { continue };
            let active = is_active(model, &cmd.id);
            push_icon_button(&mut children, cmd, active, cx, cy);
            cx += BTN + SPACING;
        }
        // Undo the trailing per-button spacing so the group extent is exact.
        if !group.is_empty() {
            cx -= SPACING;
        }
    }

    if trailing > 0 && cmd_w > 0.0 {
        push_separator(&mut children, cx, tray_h);
        cx += SEP_GAP;
    }
    for w in model.pen_widths {
        let active = (model.pen_width - *w).abs() < f64::EPSILON;
        children.push(width_chip(*w, active, cx, cy));
        cx += CHIP_W + SPACING;
    }
    if !model.pen_widths.is_empty() {
        cx -= SPACING;
        if !model.pen_palette.is_empty() {
            cx += SPACING;
        }
    }
    for color in model.pen_palette {
        children.push(color_swatch(color, color == model.selected_color, cx, cy));
        cx += SWATCH_W + SPACING;
    }

    Widget::Container(Container {
        id: "toolbar".to_string(),
        x: tx,
        y: ty,
        w: tray_w,
        h: tray_h,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

/// The main-axis extent of a row of `n` boxes of `box_w`, with `SPACING` between.
fn row_extent(n: usize, box_w: f64) -> f64 {
    if n == 0 {
        0.0
    } else {
        n as f64 * box_w + (n - 1) as f64 * SPACING
    }
}

/// The frosted tray body: a `material` fill + a `hairline` border, the single
/// rounded surface every button sits on.
fn tray_body(w: f64, h: f64) -> Widget {
    Widget::Rect(Rect {
        id: "toolbar::tray".to_string(),
        x: 0.0,
        y: 0.0,
        w,
        h,
        style: RectStyle {
            fill: Some(Paint::Token("material".to_string())),
            stroke: Some((Paint::Token("hairline".to_string()), 1.0)),
            corner_radius: TRAY_RADIUS,
            opacity: 1.0,
        },
        hoverable: false,
    })
}

/// A `hairline` vertical separator centered in a `SEP_GAP` slot starting at `cx`,
/// inset from the tray's top/bottom padding. Non-interactive (an inert id).
fn push_separator(out: &mut Vec<Widget>, cx: f64, tray_h: f64) {
    let inset = PADDING + 2.0;
    out.push(Widget::Rect(Rect {
        id: format!("toolbar::sep@{cx}"),
        x: cx + (SEP_GAP - SEP_W) / 2.0,
        y: inset,
        w: SEP_W,
        h: tray_h - inset * 2.0,
        style: RectStyle {
            fill: Some(Paint::Token("hairline".to_string())),
            stroke: None,
            corner_radius: 0.0,
            opacity: 1.0,
        },
        hoverable: false,
    }));
}

/// An icon-only command button: a `cmd:<id>` hit body (active → `accent-soft`,
/// inactive → fill-less so only the frosted tray shows) + an icon glyph painted on
/// top (`text` tint, active → `selection-ring` accent). The body is a bare `Rect`
/// (no text label — the glyph carries the meaning), keeping the `cmd:<id>` id the
/// hit + resolve target; the glyph carries `cmd:<id>::icon` and is non-hittable, so
/// the press still round-trips. Falls back to a label-less body if the id somehow
/// has no registry glyph (the coverage test rules that out).
fn push_icon_button(out: &mut Vec<Widget>, cmd: &ObjectCommand, active: bool, x: f64, y: f64) {
    let body_id = format!("{CMD_PREFIX}{}", cmd.id);
    out.push(Widget::Rect(Rect {
        id: body_id.clone(),
        x,
        y,
        w: BTN,
        h: BTN,
        style: RectStyle {
            fill: active.then(|| Paint::Token("accent-soft".to_string())),
            stroke: None,
            corner_radius: BTN_RADIUS,
            opacity: 1.0,
        },
        // An idle icon button is borderless/fill-less; pointing at it takes the
        // `hover` background (the projection swaps it), so the button still reads.
        hoverable: true,
    }));
    let tint = if active { "selection-ring" } else { "text" };
    if let Some(icon) =
        icon_in_box(format!("{body_id}::icon"), &cmd.id, x, y, BTN, BTN, Paint::Token(tint.to_string()))
    {
        out.push(icon);
    }
}

/// A brush-size chip bound to a pen width (`pen-width:<px>`). `active` tints it with
/// the `accent-soft` body. The label is the integer px so the chip self-describes.
fn width_chip(width_px: f64, active: bool, x: f64, y: f64) -> Widget {
    Widget::Button(Button {
        id: format!("{PEN_WIDTH_PREFIX}{width_px}"),
        x,
        y,
        w: CHIP_W,
        h: BTN,
        // Rust's f64 Display already omits a `.0` (a whole width reads `2`, a
        // fractional `1.5`), so no width-narrowing int cast is needed for the label.
        label: format!("{width_px}"),
        style: button_style(active),
        label_size_px: 11.0,
        label_color: TextPaint::Token("text".to_string()),
    })
}

/// A palette color chip bound to a pen-color selection (`swatch:<hex>`). The fill is
/// the LITERAL color (a pen color is not a theme token); `selected` rings it.
fn color_swatch(hex: &str, selected: bool, x: f64, y: f64) -> Widget {
    Widget::Swatch(Swatch {
        id: format!("{SWATCH_PREFIX}{hex}"),
        x,
        y,
        w: SWATCH_W,
        h: BTN,
        fill: Paint::Solid(hex.to_string()),
        selected,
    })
}

/// Whether a command id reads active. Tool commands compare to `active_tool`;
/// insert commands to `create_kind`; the two toggle entry points
/// (`open-template-library`/`toggle-diagnostics`) to their open flags; everything
/// else is never active.
fn is_active(model: &UiModel, command_id: &str) -> bool {
    match command_id {
        "open-template-library" => return model.template_open,
        "toggle-diagnostics" => return model.diagnostics_open,
        _ => {}
    }
    match category(model.command_catalog, command_id) {
        Some(ObjectCommandCategory::Tool) => model.active_tool == command_id,
        Some(ObjectCommandCategory::Insert) => model.create_kind == Some(command_id),
        _ => false,
    }
}

fn command<'a>(catalog: &'a [ObjectCommand], id: &str) -> Option<&'a ObjectCommand> {
    catalog.iter().find(|c| c.id == id)
}

fn category(catalog: &[ObjectCommand], id: &str) -> Option<ObjectCommandCategory> {
    command(catalog, id).map(|c| c.category)
}
