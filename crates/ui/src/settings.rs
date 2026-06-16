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
    Axis, Container, CrossAlign, Edges, Paint, Rect, RectStyle, Text, TextPaint, Widget,
};

use crate::composites;
use crate::UiModel;

/// The full-viewport background scrim id — a press dismisses the modal
/// (`intent::resolve` maps it to `Dismiss`). Distinct from the context menu's scrim.
pub(crate) const SCRIM_ID: &str = "settings::scrim";

const PANEL_W: f64 = 420.0;
const PADDING: f64 = 20.0;
const TITLE_H: f64 = 28.0;
const NOTE_H: f64 = 32.0;
const SECTION_TITLE_H: f64 = 22.0;
const ROW_H: f64 = 24.0;
const SECTION_GAP: f64 = 10.0;

/// Build the modal: a full-viewport scrim + a centered panel listing the command
/// catalog (grouped by category in order) then the gesture catalog. Read-only.
pub(crate) fn build(model: &UiModel) -> Widget {
    let (vw, vh) = model.viewport;

    let mut rows: Vec<Widget> = Vec::new();
    let mut y = PADDING;
    rows.push(title("settings::title", "Keyboard Shortcuts", y));
    y += TITLE_H;
    rows.push(note(y));
    y += NOTE_H + SECTION_GAP;

    // Commands grouped by category in first-seen (catalog) order — the same grouping
    // the Svelte modal did, now reading the catalog directly.
    for (category, commands) in group_by_category(model.command_catalog) {
        rows.push(section_title(category_label(category), y));
        y += SECTION_TITLE_H;
        for cmd in commands {
            rows.push(command_row(cmd, model.is_mac, y));
            y += ROW_H;
        }
        y += SECTION_GAP;
    }

    // The hold-key gesture catalog (the second section).
    if !model.gesture_catalog.is_empty() {
        rows.push(section_title("Gestures", y));
        y += SECTION_TITLE_H;
        for gesture in model.gesture_catalog {
            rows.push(gesture_row(gesture, model.is_mac, y));
            y += ROW_H;
        }
        y += SECTION_GAP;
    }

    let panel_h = y + PADDING - SECTION_GAP;
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
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![
            scrim(vw, vh),
            panel(panel_x, panel_y, panel_h, rows),
        ],
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

/// The centered material panel holding the offset rows.
fn panel(x: f64, y: f64, h: f64, rows: Vec<Widget>) -> Widget {
    // The macOS-material panel: soft-shadow underlay + a frosted `material` body with
    // a `hairline` border (radius 14), then the offset rows on top.
    let mut children = Vec::new();
    composites::material_panel(&mut children, "settings", PANEL_W, h, 14.0);
    children.extend(rows.into_iter().map(|w| offset(w, PADDING, 0.0)));
    Widget::Container(Container {
        id: "settings::panel".to_string(),
        x,
        y,
        w: PANEL_W,
        h,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children,
    })
}

fn title(id: &str, label: &str, y: f64) -> Widget {
    Widget::Text(Text {
        id: id.to_string(),
        x: 0.0,
        y,
        w: PANEL_W - PADDING * 2.0,
        h: TITLE_H,
        label: label.to_string(),
        size_px: 16.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    })
}

fn note(y: f64) -> Widget {
    Widget::Text(Text {
        id: "settings::note".to_string(),
        x: 0.0,
        y,
        w: PANEL_W - PADDING * 2.0,
        h: NOTE_H,
        label: "Shortcuts are read-only here.".to_string(),
        size_px: 11.0,
        // The note is a muted caption (`text-secondary`); the title stays primary.
        color: TextPaint::Token("text-secondary".to_string()),
        align_center: false,
    })
}

fn section_title(label: &str, y: f64) -> Widget {
    Widget::Text(Text {
        id: format!("settings::section::{}", label.to_lowercase()),
        x: 0.0,
        y,
        w: PANEL_W - PADDING * 2.0,
        h: SECTION_TITLE_H,
        label: label.to_string(),
        size_px: 13.0,
        color: TextPaint::Token("text".to_string()),
        align_center: false,
    })
}

/// One command row: a left label + a right-aligned kbd pill (or an em-dash when the
/// command has no default shortcut). Non-interactive.
fn command_row(cmd: &ObjectCommand, is_mac: bool, y: f64) -> Widget {
    let kbd = cmd
        .default_shortcut
        .as_deref()
        .map(|s| format_shortcut(s, is_mac))
        .unwrap_or_else(|| "—".to_string());
    row(&format!("settings::cmd::{}", cmd.id), &cmd.label, &kbd, y)
}

/// One gesture row: a left label + a right-aligned "Hold X" pill.
fn gesture_row(gesture: &ObjectGesture, is_mac: bool, y: f64) -> Widget {
    row(
        &format!("settings::gesture::{}", gesture.id),
        &gesture.label,
        &format_gesture_trigger(gesture, is_mac),
        y,
    )
}

fn row(id: &str, label: &str, kbd: &str, y: f64) -> Widget {
    let label_w = (PANEL_W - PADDING * 2.0) * 0.62;
    let kbd_w = PANEL_W - PADDING * 2.0 - label_w;
    Widget::Container(Container {
        id: format!("{id}::row"),
        x: 0.0,
        y,
        w: PANEL_W - PADDING * 2.0,
        h: ROW_H,
        direction: Axis::None,
        spacing: 0.0,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        children: vec![
            Widget::Text(Text {
                id: format!("{id}::label"),
                x: 0.0,
                y: 0.0,
                w: label_w,
                h: ROW_H,
                label: label.to_string(),
                size_px: 12.0,
                color: TextPaint::Token("text".to_string()),
                align_center: false,
            }),
            // The kbd pill: a muted rounded rect + a centered token glyph.
            Widget::Rect(Rect {
                id: format!("{id}::kbd-bg"),
                x: label_w,
                y: 2.0,
                w: kbd_w,
                h: ROW_H - 4.0,
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
                x: label_w,
                y: 0.0,
                w: kbd_w,
                h: ROW_H,
                label: kbd.to_string(),
                size_px: 11.0,
                color: TextPaint::Token("text".to_string()),
                align_center: true,
            }),
        ],
    })
}

/// Wrap a widget in an offset absolute container (rows are positioned at panel-local
/// (PADDING, y) inside the panel).
fn offset(child: Widget, dx: f64, dy: f64) -> Widget {
    Widget::Container(Container {
        id: format!("{}::soff", child_id(&child)),
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
        HoldInput::Button => format!("{} Button", capitalize(trigger.button.as_deref().unwrap_or(""))),
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
