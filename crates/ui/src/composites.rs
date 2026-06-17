//! Inspector composites built ONLY from ui-core primitives — no new ui-core
//! widget. Align9, Lanes, NumberField, PaintField, Badge are the catalog widgets
//! ui-core lacks; each is assembled here so ui-core stays minimal (the
//! "no abstraction past a present caller" rule — these have one caller, the
//! inspector). Every fill/stroke is a Token paint, so a theme flip recolors with
//! zero rebake.
//!
//! Interactive parts carry `insp:`-prefixed ids whose suffix encodes the wire
//! value the op expects, so `intent::resolve` decodes a press into an
//! `InspectorEdit` without a second value source. The align grid in particular
//! embeds `{main,cross}` in the id (`insp:align:main=center,cross=end`) because a
//! `Pressed` action carries only the widget id.

use shape_ui_core::{
    Axis, Button, Container, CrossAlign, Edges, MainAlign, Paint, Rect, RectStyle, Segment, Swatch,
    Text, TextInput, TextPaint, Toggle, Widget, SPACE_SM, SPACE_XS,
};

use crate::inspector::INSPECTOR_PREFIX;

/// The macOS-material panel language shared by every elevated surface (inspector,
/// settings, diagnostics, template popup, toolbar tray): a soft-shadow underlay (one
/// translucent black rect, theme-independent so a flip neither rebakes geometry nor
/// changes the fill repr) → a frosted `material` body with a `hairline` 1px border.
/// `out` receives the shadow + body in paint order (shadow behind, body on top); the
/// panel's own content is pushed by the caller AFTER. `radius` is the body corner.
pub(crate) fn material_panel(out: &mut Vec<Widget>, prefix: &str, w: f64, h: f64, radius: f64) {
    soft_shadow(out, prefix, w, h, radius);
    out.push(Widget::Rect(Rect {
        id: format!("{prefix}::bg"),
        x: 0.0,
        y: 0.0,
        w,
        h,
        style: RectStyle {
            fill: Some(Paint::Token("material".to_string())),
            stroke: Some((Paint::Token("hairline".to_string()), 1.0)),
            corner_radius: radius,
            opacity: 1.0,
        },
        hoverable: false,
    }));
}

/// The soft-elevation seat UNDER a panel of `w`×`h`: ONE translucent black rounded
/// rect, slightly larger and dropped down a touch, fill-only (no stroke). A single
/// low-alpha layer reads as a soft seat without the concentric rings a multi-layer
/// stack of differently-offset rects produced; the renderer's own per-object Gaussian
/// shadow pass supplies the actual softness. The black `Solid` is theme-independent,
/// so a theme flip neither rebakes geometry nor changes the fill repr — the
/// zero-rebake bar.
pub(crate) fn soft_shadow(out: &mut Vec<Widget>, prefix: &str, w: f64, h: f64, radius: f64) {
    const SPREAD: f64 = 3.0;
    const DROP: f64 = 5.0;
    out.push(Widget::Rect(Rect {
        id: format!("{prefix}::shadow"),
        x: -SPREAD,
        y: -SPREAD + DROP,
        w: w + SPREAD * 2.0,
        h: h + SPREAD * 2.0,
        style: RectStyle {
            fill: Some(Paint::Solid("#000000".to_string())),
            stroke: None,
            corner_radius: radius + SPREAD,
            opacity: 0.12,
        },
        hoverable: false,
    }));
}

/// A read-only label pill (the inspector `Badge`, e.g. the role label). A muted
/// rounded rect body + a centered Text; non-interactive (no `insp:` id).
pub(crate) fn badge(id: &str, label: &str, x: f64, y: f64, w: f64, h: f64) -> Widget {
    Widget::Container(Container {
        id: format!("{id}::badge"),
        x,
        y,
        w,
        h,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children: vec![
            Widget::Rect(shape_ui_core::Rect {
                id: format!("{id}::badge-bg"),
                x: 0.0,
                y: 0.0,
                w,
                h,
                style: RectStyle {
                    fill: Some(Paint::Token("surface-muted".to_string())),
                    stroke: None,
                    corner_radius: h / 2.0,
                    opacity: 1.0,
                },
                hoverable: false,
            }),
            Widget::Text(Text {
                id: format!("{id}::badge-label"),
                x: 0.0,
                y: 0.0,
                w,
                h,
                label: label.to_string(),
                size_px: 12.0,
                color: TextPaint::Token("text".to_string()),
                align_center: true,
            }),
        ],
    })
}

/// A numeric field: a TextInput (the value) + a trailing unit Text. The input id is
/// `insp:<control-id>` so a commit maps to the control's edit; the unit label is
/// inert. `value` is the current px value as a display string (empty when mixed).
pub(crate) fn number_field(
    control_id: &str,
    value: &str,
    unit: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Widget {
    let unit_w = 28.0;
    let input_w = (w - unit_w - SPACE_XS).max(0.0);
    Widget::Container(Container {
        id: format!("{control_id}::number"),
        x,
        y,
        w,
        h,
        direction: Axis::Horizontal,
        spacing: SPACE_XS,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Center,
        clip: false,
        children: vec![
            text_input(control_id, value, "", input_w, h),
            Widget::Text(Text {
                id: format!("{control_id}::unit"),
                x: 0.0,
                y: 0.0,
                w: unit_w,
                h,
                label: unit.to_string(),
                size_px: 12.0,
                // The trailing unit reads muted (`text-secondary`), like the labels.
                color: TextPaint::Token("text-secondary".to_string()),
                align_center: false,
            }),
        ],
    })
}

/// A paint field: a solid-color Swatch + a hex TextInput. The swatch is inert
/// display; the hex input id is `insp:<control-id>` so a commit authors a solid
/// paint (`{kind:solid,color}` in `resolve`). `hex` is the current solid color or
/// empty (gradient/token/unset/mixed read empty).
pub(crate) fn paint_field(control_id: &str, hex: &str, x: f64, y: f64, w: f64, h: f64) -> Widget {
    let swatch_w = h;
    let input_w = (w - swatch_w - SPACE_SM).max(0.0);
    // An empty hex shows a neutral surface swatch; a present hex shows it literally.
    let fill = if hex.is_empty() {
        Paint::Token("surface-muted".to_string())
    } else {
        Paint::Solid(hex.to_string())
    };
    Widget::Container(Container {
        id: format!("{control_id}::paint"),
        x,
        y,
        w,
        h,
        direction: Axis::Horizontal,
        spacing: SPACE_SM,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Center,
        clip: false,
        children: vec![
            Widget::Swatch(Swatch {
                id: format!("{control_id}::paint-swatch"),
                x: 0.0,
                y: 0.0,
                w: swatch_w,
                h,
                fill,
                selected: false,
            }),
            text_input(control_id, hex, "#rrggbb", input_w, h),
        ],
    })
}

/// A lanes field: a count TextInput + a Fill Toggle. When Fill is on the count is
/// blanked (the catalog `Lanes::Fill` carries no count). The toggle id is
/// `insp:<control-id>::fill` and the count id `insp:<control-id>::count`, both
/// decoded by `resolve` into the lanes wire value.
pub(crate) fn lanes_field(
    control_id: &str,
    count: Option<i64>,
    fill: bool,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Widget {
    let toggle_w = 48.0;
    let count_w = (w - toggle_w - SPACE_SM).max(0.0);
    let count_str = if fill {
        String::new()
    } else {
        count.map(|c| c.to_string()).unwrap_or_default()
    };
    Widget::Container(Container {
        id: format!("{control_id}::lanes"),
        x,
        y,
        w,
        h,
        direction: Axis::Horizontal,
        spacing: SPACE_SM,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Center,
        clip: false,
        children: vec![
            text_input(&format!("{control_id}::count"), &count_str, "1", count_w, h),
            Widget::Toggle(Toggle {
                id: format!("{INSPECTOR_PREFIX}{control_id}::fill"),
                x: 0.0,
                y: 0.0,
                w: toggle_w,
                h: h.min(24.0),
                on: fill,
            }),
        ],
    })
}

/// The 3×3 alignment grid (main on columns, cross on rows) + the two values the
/// grid can't express (`spaceBetween` main, `stretch` cross) as trailing toggles.
/// Each cell is a Swatch whose id encodes its `{main,cross}` so a press resolves to
/// the exact align value (`insp:align:main=center,cross=end`). `main`/`cross` are
/// the current values (so the matching cell shows selected).
pub(crate) fn align9(control_id: &str, main: &str, cross: &str, x: f64, y: f64) -> Widget {
    const GRID: [&str; 3] = ["start", "center", "end"];
    let cell = 18.0;
    let gap = 3.0;
    let grid_extent = cell * 3.0 + gap * 2.0;
    let mut children: Vec<Widget> = Vec::new();
    for (row, c) in GRID.iter().enumerate() {
        for (col, m) in GRID.iter().enumerate() {
            let cx = col as f64 * (cell + gap);
            let cy = row as f64 * (cell + gap);
            let selected = main == *m && cross == *c;
            // The selected cell tints `accent-soft` (the active-control language);
            // an idle cell stays `surface-muted`. The Swatch's own `selected` ring
            // still marks the choice for the press round-trip.
            let fill = if selected {
                "accent-soft"
            } else {
                "surface-muted"
            };
            children.push(Widget::Swatch(Swatch {
                id: format!("{INSPECTOR_PREFIX}{control_id}:main={m},cross={c}"),
                x: cx,
                y: cy,
                w: cell,
                h: cell,
                fill: Paint::Token(fill.to_string()),
                selected,
            }));
        }
    }
    // spaceBetween (main) + stretch (cross): two segmented-style toggle buttons.
    let toggle_y = grid_extent + 6.0;
    children.push(Widget::Button(Button {
        id: format!("{INSPECTOR_PREFIX}{control_id}:main=spaceBetween,cross={cross}"),
        x: 0.0,
        y: toggle_y,
        w: grid_extent,
        h: 22.0,
        label: "Space between".to_string(),
        style: button_style(main == "spaceBetween"),
        label_size_px: 11.0,
        label_color: TextPaint::Token("text".to_string()),
    }));
    children.push(Widget::Button(Button {
        id: format!("{INSPECTOR_PREFIX}{control_id}:main={main},cross=stretch"),
        x: 0.0,
        y: toggle_y + 26.0,
        w: grid_extent,
        h: 22.0,
        label: "Stretch".to_string(),
        style: button_style(cross == "stretch"),
        label_size_px: 11.0,
        label_color: TextPaint::Token("text".to_string()),
    }));
    Widget::Container(Container {
        id: format!("{control_id}::align"),
        x,
        y,
        w: grid_extent,
        h: toggle_y + 26.0 + 22.0,
        direction: Axis::None,
        spacing: 0.0,
        main_align: MainAlign::Start,
        padding: Edges::all(0.0),
        align: CrossAlign::Start,
        clip: false,
        children,
    })
}

/// A segmented control whose id is `insp:<control-id>` so a cell change resolves to
/// the control's segment edit. `selected` is the active cell index.
pub(crate) fn segment_field(
    control_id: &str,
    labels: Vec<String>,
    selected: usize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Widget {
    Widget::Segment(Segment {
        id: format!("{INSPECTOR_PREFIX}{control_id}"),
        x,
        y,
        w,
        h,
        labels,
        selected,
        label_size_px: 12.0,
        label_color: TextPaint::Token("text".to_string()),
    })
}

/// An `insp:`-prefixed single-line text field (the common base for Name/Number/
/// Paint-hex). The id is `insp:<id>` so a commit routes to the inspector edit.
pub(crate) fn text_input(id: &str, value: &str, placeholder: &str, w: f64, h: f64) -> Widget {
    Widget::TextInput(TextInput {
        id: format!("{INSPECTOR_PREFIX}{id}"),
        x: 0.0,
        y: 0.0,
        w,
        h,
        value: value.to_string(),
        focused: false,
        size_px: 12.0,
        color: TextPaint::Token("text".to_string()),
        placeholder: placeholder.to_string(),
    })
}

/// A toggle whose id is `insp:<control-id>` so a flip resolves to the control's
/// boolean edit (visible/locked/clip).
pub(crate) fn toggle_field(control_id: &str, on: bool, x: f64, y: f64) -> Widget {
    Widget::Toggle(Toggle {
        id: format!("{INSPECTOR_PREFIX}{control_id}"),
        x,
        y,
        w: 48.0,
        h: 24.0,
        on,
    })
}

/// The canonicalize Action button (`insp:<control-id>`, fires `InspectorAction`).
/// Unlike a borderless toolbar/chrome icon button, an action button reads as a solid
/// pill so it stands apart from the frosted panel: an OPAQUE `surface-muted` fill
/// (theme-aware — light `e9e9eb`, dark `3a3a3c`). The shared `button_style(false)`
/// left it fill-less, which showed the light panel `material` through and read as a
/// near-white washed button in dark (the QA dark-contrast defect); the opaque token
/// flips to a legible dark pill.
pub(crate) fn action_button(control_id: &str, label: &str, x: f64, y: f64, w: f64) -> Widget {
    Widget::Button(Button {
        id: format!("{INSPECTOR_PREFIX}{control_id}"),
        x,
        y,
        w,
        h: 28.0,
        label: label.to_string(),
        style: RectStyle {
            fill: Some(Paint::Token("surface-muted".to_string())),
            stroke: None,
            corner_radius: 8.0,
            opacity: 1.0,
        },
        label_size_px: 12.0,
        label_color: TextPaint::Token("text".to_string()),
    })
}

/// A button visual in the macOS material language: borderless and fill-less at rest
/// (the frosted panel shows through), an `accent-soft` tint when active, radius 8.
/// Hover feedback is the projection's `hover` swap, so no resting border/fill is
/// needed — that is what removes the boxes-in-boxes wireframe look.
pub(crate) fn button_style(active: bool) -> RectStyle {
    RectStyle {
        fill: active.then(|| Paint::Token("accent-soft".to_string())),
        stroke: None,
        corner_radius: 8.0,
        opacity: 1.0,
    }
}
