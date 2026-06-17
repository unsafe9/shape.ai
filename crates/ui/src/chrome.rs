//! Top-level chrome: the theme-toggle button and the canvas switcher. The theme
//! toggle binds to the `toggle-theme` command-catalog entry (id `cmd:toggle-theme`),
//! so a press resolves to `Intent::Command("toggle-theme")` and self-documents in
//! the settings modal, closing the single-source gap the raw Svelte theme button
//! left open. The switcher's canvas/new/delete controls are shell/runtime concerns
//! (no object command), so they carry their own intent prefixes, not `cmd:`.

use shape_ui_core::{
    Axis, Button, Container, CrossAlign, Edges, MainAlign, Paint, Rect, RectStyle, Text, TextPaint,
    Widget,
};

use crate::composites::button_style;
use crate::icons::icon_in_box;
use crate::toolbar::CMD_PREFIX;
use crate::UiModel;

/// The id namespace binding a switcher tab to a canvas activation (`canvas:<id>`).
pub(crate) const CANVAS_PREFIX: &str = "canvas:";
/// The id namespace binding the switcher's delete button to a canvas (`canvas-delete:<id>`).
pub(crate) const CANVAS_DELETE_PREFIX: &str = "canvas-delete:";
/// The switcher's New-canvas button id (bare — it carries no target).
pub(crate) const CANVAS_NEW_ID: &str = "canvas-new";

const SIZE: f64 = 36.0;
const MARGIN: f64 = 16.0;

/// The theme-toggle button, anchored top-left of the viewport. An icon chrome button:
/// a `cmd:toggle-theme` material body (so a press resolves to the catalog command and
/// self-documents in settings) under the registry `toggle-theme` glyph (the single
/// sun/moon mark that reads in both themes — the icon is theme-independent, so the
/// flip neither rebakes geometry nor swaps the glyph the way the old ☀/☾ label did).
pub(crate) fn theme_toggle(_model: &UiModel) -> Widget {
    icon_chrome_button(
        &format!("{CMD_PREFIX}toggle-theme"),
        "toggle-theme",
        MARGIN,
        MARGIN,
        SIZE,
    )
}

/// An icon-only chrome button: a `material` body (the `id` hit/resolve target, drawn
/// with the shared `button_style`) under a registry glyph centered in the box (the
/// glyph carries `<id>::icon`, painted with the `text` token; non-hittable, so the
/// press round-trips off the body id — the toolbar icon-button idiom, reused for the
/// standalone chrome controls). `glyph_id` is the registry key; falls back to a bare
/// body if no glyph is registered (the coverage test rules that out).
fn icon_chrome_button(id: &str, glyph_id: &str, x: f64, y: f64, size: f64) -> Widget {
    let mut children = vec![Widget::Rect(Rect {
        id: id.to_string(),
        x: 0.0,
        y: 0.0,
        w: size,
        h: size,
        style: button_style(false),
        // A borderless icon chrome button takes the `hover` background when pointed at.
        hoverable: true,
    })];
    if let Some(icon) = icon_in_box(
        format!("{id}::icon"),
        glyph_id,
        0.0,
        0.0,
        size,
        size,
        Paint::Token("text".to_string()),
    ) {
        children.push(icon);
    }
    Widget::Container(Container {
        id: format!("{id}::button"),
        x,
        y,
        w: size,
        h: size,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}

/// The decorative bottom-right brand watermark (the `shape.ai` mark Toolbar/App
/// carried). Non-interactive (no actionable id); a single muted Text. It is the
/// idle/empty affordance — the canvas shows it under an empty scene.
pub(crate) fn watermark(model: &UiModel) -> Widget {
    let (vw, vh) = model.viewport;
    let w = 96.0;
    // The box is taller than the 13px run so the `p` descender and cap height both
    // clear the valign-middle line box — a snug 20px box top-clipped the mark.
    let h = 24.0;
    Widget::Text(Text {
        id: "watermark".to_string(),
        x: (vw - w - MARGIN).max(0.0),
        y: (vh - h - MARGIN).max(0.0),
        w,
        h,
        label: "shape.ai".to_string(),
        size_px: 13.0,
        // The idle brand mark reads as a muted caption (`text-secondary`).
        color: TextPaint::Token("text-secondary".to_string()),
        align_center: false,
    })
}

const SWITCH_H: f64 = 30.0;
const TAB_W: f64 = 110.0;
const CTRL_W: f64 = 30.0;
const GAP: f64 = 6.0;
const CONN_W: f64 = 22.0;

/// The top-left canvas switcher: a connection glyph (Wifi/WifiOff), one
/// `canvas:<id>` tab per canvas (the active one ring-marked), a `canvas-new` New
/// button, and a `canvas-delete:<active>` Delete button (omitted when only one
/// canvas remains, mirroring the Svelte disabled-when-≤1 rule). Anchored to the
/// right of the theme toggle. Tabs are core-owned buttons — the screen-space UI has
/// no OS `<select>`, so a row of marked tabs IS the switcher.
pub(crate) fn canvas_switcher(model: &UiModel) -> Widget {
    let x0 = MARGIN + SIZE + GAP;
    let mut children: Vec<Widget> = Vec::new();
    let mut cx = 0.0;

    // The connection indicator: a small dot, green online / muted offline. A literal
    // status color (not a theme token) so it reads the same in both themes.
    children.push(Widget::Rect(Rect {
        id: "canvas-switcher::conn".to_string(),
        x: cx,
        y: (SWITCH_H - 12.0) / 2.0,
        w: 12.0,
        h: 12.0,
        style: RectStyle {
            fill: Some(Paint::Solid(
                if model.connection_online {
                    "#2ea043"
                } else {
                    "#8b949e"
                }
                .to_string(),
            )),
            stroke: None,
            corner_radius: 6.0,
            opacity: 1.0,
        },
        hoverable: false,
    }));
    cx += CONN_W;

    for canvas in model.canvases {
        children.push(Widget::Button(Button {
            id: format!("{CANVAS_PREFIX}{}", canvas.id),
            x: cx,
            y: 0.0,
            w: TAB_W,
            h: SWITCH_H,
            label: canvas.title.clone(),
            style: tab_style(canvas.id == model.active_canvas_id),
            label_size_px: 11.0,
            label_color: TextPaint::Token("text".to_string()),
        }));
        cx += TAB_W + GAP;
    }

    // New canvas (always available); Delete the active canvas (only when >1 remains,
    // and never while a canvas op is in flight). Each is an icon control (the `+`/`🗑`
    // glyphs become the registry `canvas-new`/`canvas-delete` icons).
    children.push(control_button(CANVAS_NEW_ID, "canvas-new", cx));
    cx += CTRL_W + GAP;
    if model.canvases.len() > 1 && !model.canvas_busy {
        children.push(control_button(
            &format!("{CANVAS_DELETE_PREFIX}{}", model.active_canvas_id),
            "canvas-delete",
            cx,
        ));
        cx += CTRL_W;
    }

    Widget::Container(Container {
        id: "canvas-switcher".to_string(),
        x: x0,
        y: MARGIN,
        w: cx,
        h: SWITCH_H,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}

/// A small square switcher control (New / Delete), as an icon chrome button. `id` is
/// the hit/resolve target (`canvas-new` / `canvas-delete:<active>`); `glyph_id` is the
/// registry key for the centered icon. The busy-disable is decided by the caller (it
/// just omits a disabled control). Local y is 0 inside the switcher row.
fn control_button(id: &str, glyph_id: &str, x: f64) -> Widget {
    icon_chrome_button(id, glyph_id, x, 0.0, SWITCH_H)
}

/// The active switcher tab reads as a solid pill: an OPAQUE `surface-muted` fill
/// (theme-aware — light `e9e9eb`, dark `3a3a3c`) marked active by a `selection-ring`
/// stroke. The translucent `accent-soft` the shared `button_style` used was a low-
/// alpha blue wash that read as a pale near-white pill over the dark tray (the QA
/// dark-contrast defect); an opaque theme token flips to a legible dark pill instead.
/// An inactive tab stays fill-less so the chrome shows through.
fn tab_style(active: bool) -> RectStyle {
    if active {
        RectStyle {
            fill: Some(Paint::Token("surface-muted".to_string())),
            stroke: Some((Paint::Token("selection-ring".to_string()), 1.0)),
            corner_radius: 8.0,
            opacity: 1.0,
        }
    } else {
        button_style(false)
    }
}
