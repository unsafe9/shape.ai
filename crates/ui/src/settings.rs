//! The settings modal (Cmd+,): the canonical proof of the single-source rule. It
//! renders `object_command_catalog()` (grouped by category in catalog order) and
//! `object_gesture_catalog()` READ-ONLY — registering a command/gesture in
//! scene-core makes it appear here with zero edits in this file. The only Intent it
//! emits is `Dismiss` (a scrim press); no row is actuable.
//!
//! Shortcut display (`Mod`→`⌘`/`Ctrl`, etc.) is a pure formatter taking `is_mac`
//! from the model — the cores read no platform; the shell feeds the bit.

use shape_scene_core::object::catalog::commands::{ObjectCommand, ObjectCommandCategory};
use shape_scene_core::object::catalog::gestures::{HoldInput, ObjectGesture};
use shape_ui_core::{
    measure, Axis, Container, CrossAlign, Edges, MainAlign, Paint, Rect, RectStyle, Text, TextPaint,
    Widget, PANEL_RADIUS, SPACE_LG, SPACE_MD, SPACE_XS,
};

use crate::composites;
use crate::UiModel;

/// The full-viewport background scrim id — a press dismisses the modal
/// (`intent::resolve` maps it to `Dismiss`). Distinct from the context menu's scrim.
pub(crate) const SCRIM_ID: &str = "settings::scrim";

const PANEL_W: f64 = 420.0;
const PADDING: f64 = SPACE_LG + SPACE_XS;
const TITLE_H: f64 = 28.0;
const NOTE_H: f64 = 24.0;
const SECTION_TITLE_H: f64 = 22.0;
const ROW_H: f64 = 24.0;
/// The panel content width every row spans (panel minus its two side margins).
const CONTENT_W: f64 = PANEL_W - PADDING * 2.0;

/// Build the modal: a full-viewport scrim + a centered panel listing the command
/// catalog (grouped by category in order) then the gesture catalog. Read-only. The
/// panel content is one VERTICAL flex stack of blocks (`title`, `note`, one
/// per-category `section`) with a uniform `SPACE_MD` between them — the engine emits
/// the gaps, so there is no per-row `y` cursor to drift.
pub(crate) fn build(model: &UiModel) -> Widget {
    let (vw, vh) = model.viewport;

    let mut blocks: Vec<Widget> = Vec::new();
    blocks.push(title("settings::title", "Keyboard Shortcuts", TITLE_H));
    blocks.push(Widget::Text(Text {
        id: "settings::note".to_string(),
        x: 0.0,
        y: 0.0,
        w: CONTENT_W,
        h: NOTE_H,
        label: "Shortcuts are read-only here.".to_string(),
        size_px: 11.0,
        // The note is a muted caption (`text-secondary`); the title stays primary.
        color: TextPaint::Token("text-secondary".to_string()),
        align_center: false,
    }));

    // Commands grouped by category in first-seen (catalog) order — the same grouping
    // the Svelte modal did, now reading the catalog directly.
    for (category, commands) in group_by_category(model.command_catalog) {
        let rows = commands
            .iter()
            .map(|cmd| command_row(cmd, model.is_mac))
            .collect();
        blocks.push(section(category_label(category), rows));
    }

    // The hold-key gesture catalog (the second section).
    if !model.gesture_catalog.is_empty() {
        let rows = model
            .gesture_catalog
            .iter()
            .map(|g| gesture_row(g, model.is_mac))
            .collect();
        blocks.push(section("Gestures", rows));
    }

    let panel_h = stack_height(&blocks, SPACE_MD) + PADDING * 2.0;
    let panel_x = ((vw - PANEL_W) / 2.0).max(0.0);
    let panel_y = ((vh - panel_h) / 2.0).max(0.0);

    Widget::Container(Container {
        id: "settings".to_string(),
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children: vec![scrim(vw, vh), panel(panel_x, panel_y, panel_h, blocks)],
    })
}

/// The laid-out height of a vertical stack of widgets: each own height + `gap`
/// between. Mirrors what the vertical flex emits, so the body rect sizes to content.
/// Heights come from the ui-core `measure` (the same one the flex lays out against),
/// so a rich row (Swatch/Toggle/Slider/…) contributes its real extent, not zero.
fn stack_height(items: &[Widget], gap: f64) -> f64 {
    items.iter().map(|w| measure(w).1).sum::<f64>()
        + (items.len().saturating_sub(1)) as f64 * gap
}

/// One titled block: a section-title Text above its rows, stacked tight (`SPACE_XS`)
/// in a vertical flex. The block's own height is the laid-out stack height, so the
/// parent block-stack can place the next block beneath it.
fn section(label: &str, rows: Vec<Widget>) -> Widget {
    let mut children = vec![section_title(label, SECTION_TITLE_H)];
    children.extend(rows);
    let h = stack_height(&children, SPACE_XS);
    Widget::Container(Container {
        id: format!("settings::sec::{}", label.to_lowercase()),
        x: 0.0,
        y: 0.0,
        w: CONTENT_W,
        h,
        direction: Axis::Vertical,
        spacing: SPACE_XS,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}

/// The full-viewport dismiss scrim (a translucent shadow-token rect). A press maps
/// to `Dismiss`.
fn scrim(vw: f64, vh: f64) -> Widget {
    Widget::Rect(Rect {
        id: SCRIM_ID.to_string(),
        x: 0.0,
        y: 0.0,
        w: vw,
        h: vh,
        style: RectStyle {
            fill: Some(Paint::Token("shadow".to_string())),
            stroke: None,
            corner_radius: 0.0,
            opacity: 1.0,
        },
        hoverable: false,
    })
}

/// The centered material panel: a soft-shadow underlay + a frosted `material` body
/// (`hairline` border, radius `PANEL_RADIUS`), then ONE padded vertical flex that
/// stacks the blocks — the flex emits both the panel inset (its padding) and the
/// `SPACE_MD` inter-block gap, replacing the old per-row absolute `offset` wrappers.
fn panel(x: f64, y: f64, h: f64, blocks: Vec<Widget>) -> Widget {
    let mut children = Vec::new();
    composites::material_panel(&mut children, "settings", PANEL_W, h, PANEL_RADIUS);
    children.push(Widget::Container(Container {
        id: "settings::content".to_string(),
        x: 0.0,
        y: 0.0,
        w: PANEL_W,
        h,
        direction: Axis::Vertical,
        spacing: SPACE_MD,
        main_align: MainAlign::Start,
        padding: Edges::all(PADDING),
        align: CrossAlign::Start,
        clip: false,
        children: blocks,
    }));
    Widget::Container(Container {
        id: "settings::panel".to_string(),
        x,
        y,
        w: PANEL_W,
        h,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}

fn title(id: &str, label: &str, h: f64) -> Widget {
    Widget::Text(Text {
        id: id.to_string(),
        x: 0.0,
        y: 0.0,
        w: CONTENT_W,
        h,
        label: label.to_string(),
        size_px: 16.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    })
}

fn section_title(label: &str, h: f64) -> Widget {
    Widget::Text(Text {
        id: format!("settings::section::{}", label.to_lowercase()),
        x: 0.0,
        y: 0.0,
        w: CONTENT_W,
        h,
        label: label.to_string(),
        size_px: 13.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    })
}

/// One command row: a left label + a right-aligned kbd pill (or an em-dash when the
/// command has no default shortcut). Non-interactive.
fn command_row(cmd: &ObjectCommand, is_mac: bool) -> Widget {
    let kbd = cmd
        .default_shortcut
        .as_deref()
        .map(|s| format_shortcut(s, is_mac))
        .unwrap_or_else(|| "—".to_string());
    row(&format!("settings::cmd::{}", cmd.id), &cmd.label, &kbd)
}

/// One gesture row: a left label + a right-aligned "Hold X" pill.
fn gesture_row(gesture: &ObjectGesture, is_mac: bool) -> Widget {
    row(
        &format!("settings::gesture::{}", gesture.id),
        &gesture.label,
        &format_gesture_trigger(gesture, is_mac),
    )
}

/// The fixed width of the trailing kbd pill — wide enough for the longest chord
/// (`⌘⇧Z`/`Ctrl+Shift+Z`); `SpaceBetween` pins it to the row's trailing edge so the
/// label and pill sit at the two inner edges with no `label_w`/`kbd_w` split math.
const KBD_W: f64 = 96.0;

/// A label|kbd-pill row laid out HORIZONTALLY with `MainAlign::SpaceBetween`: the
/// label pins to the leading edge, the kbd pill to the trailing edge, and the engine
/// (not a hand cursor) owns the gap between. Cross-centered so the pill sits on the
/// label's optical baseline regardless of their differing heights.
fn row(id: &str, label: &str, kbd: &str) -> Widget {
    Widget::Container(Container {
        id: format!("{id}::row"),
        x: 0.0,
        y: 0.0,
        w: CONTENT_W,
        h: ROW_H,
        direction: Axis::Horizontal,
        spacing: 0.0,
        main_align: MainAlign::SpaceBetween,
        padding: Edges::all(0.0),
        align: CrossAlign::Center,
        clip: false,
        children: vec![
            Widget::Text(Text {
                id: format!("{id}::label"),
                x: 0.0,
                y: 0.0,
                w: CONTENT_W - KBD_W,
                h: ROW_H,
                label: label.to_string(),
                size_px: 12.0,
                color: TextPaint::Token("text".to_string()),
                align_center: false,
            }),
            kbd_pill(id, kbd),
        ],
    })
}

/// The trailing kbd pill: a muted rounded rect with a centered token glyph stacked on
/// top (an absolute sub-container so the glyph rides the pill body). Fixed `KBD_W`
/// wide so the parent row can `SpaceBetween` it against the label.
fn kbd_pill(id: &str, kbd: &str) -> Widget {
    Widget::Container(Container {
        id: format!("{id}::kbd-pill"),
        x: 0.0,
        y: 0.0,
        w: KBD_W,
        h: ROW_H - SPACE_XS,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children: vec![
            Widget::Rect(Rect {
                id: format!("{id}::kbd-bg"),
                x: 0.0,
                y: 0.0,
                w: KBD_W,
                h: ROW_H - SPACE_XS,
                style: RectStyle {
                    fill: Some(Paint::Token("surface-muted".to_string())),
                    stroke: None,
                    corner_radius: 5.0,
                    opacity: 1.0,
                },
                hoverable: false,
            }),
            Widget::Text(Text {
                id: format!("{id}::kbd"),
                x: 0.0,
                y: 0.0,
                w: KBD_W,
                h: ROW_H - SPACE_XS,
                label: kbd.to_string(),
                size_px: 11.0,
                color: TextPaint::Token("text".to_string()),
                align_center: true,
            }),
        ],
    })
}

/// Commands grouped by category in first-seen (catalog) order. Mirrors the Svelte
/// modal's grouping over the same catalog.
fn group_by_category(
    catalog: &[ObjectCommand],
) -> Vec<(ObjectCommandCategory, Vec<&ObjectCommand>)> {
    let mut groups: Vec<(ObjectCommandCategory, Vec<&ObjectCommand>)> = Vec::new();
    for cmd in catalog {
        match groups.iter_mut().find(|(cat, _)| *cat == cmd.category) {
            Some((_, list)) => list.push(cmd),
            None => groups.push((cmd.category, vec![cmd])),
        }
    }
    groups
}

fn category_label(category: ObjectCommandCategory) -> &'static str {
    use ObjectCommandCategory::*;
    match category {
        Tool => "Tool",
        Insert => "Insert",
        Clipboard => "Clipboard",
        Edit => "Edit",
        Selection => "Selection",
        Arrange => "Arrange",
        Order => "Order",
        History => "History",
        Path => "Path",
        Style => "Style",
        Annotate => "Annotate",
        View => "View",
    }
}

/// Render a default shortcut for display, resolving the `Mod`/`Shift`/`Alt` tokens to
/// the host's symbols (the `formatShortcut` TS logic, moved into Rust; `is_mac` is
/// fed in so the cores read no platform).
pub(crate) fn format_shortcut(shortcut: &str, is_mac: bool) -> String {
    let sep = if is_mac { "" } else { "+" };
    shortcut
        .split('+')
        .map(|token| match token {
            "Mod" => if is_mac { "⌘" } else { "Ctrl" }.to_string(),
            "Shift" => if is_mac { "⇧" } else { "Shift" }.to_string(),
            "Alt" => if is_mac { "⌥" } else { "Alt" }.to_string(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(sep)
}

/// Render a hold-gesture trigger as a "Hold X" label (the `formatGestureTrigger` TS
/// logic). A `degrees` param (the coarse-rotate step) is appended as `· 15°`.
fn format_gesture_trigger(gesture: &ObjectGesture, is_mac: bool) -> String {
    let trigger = &gesture.trigger;
    let token = match trigger.input {
        HoldInput::Key => trigger.key.clone().unwrap_or_default(),
        HoldInput::Button => format!(
            "{} Button",
            capitalize(trigger.button.as_deref().unwrap_or(""))
        ),
        HoldInput::Modifier => format_shortcut(trigger.modifier.as_deref().unwrap_or(""), is_mac),
    };
    let base = format!("Hold {token}");
    match trigger.degrees {
        Some(deg) => format!("{base} · {}°", fmt_degrees(deg)),
        None => base,
    }
}

fn capitalize(token: &str) -> String {
    let mut chars = token.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Format the coarse-rotate degrees without a trailing `.0` (15.0 → "15").
fn fmt_degrees(deg: f64) -> String {
    if deg.fract() == 0.0 {
        format!("{deg:.0}")
    } else {
        format!("{deg}")
    }
}
