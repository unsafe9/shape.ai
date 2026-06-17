//! `render(tree, viewport_px, theme_dark) -> RenderObjectScene` — a screen-space
//! UI scene. Geometry is object-local quantized px; each widget rides a pure
//! translate transform to its screen-px origin under an IDENTITY camera, so the
//! object pass maps local px → screen px 1:1 (no pan/zoom).
//!
//! Fills/strokes emit `RPaint::Token` so the renderer resolves dark/light with
//! zero rebake; only TEXT resolves to a fixed hex here (RTextRun.color is a fixed
//! hex per renderer-core), via `theme::resolve_text_paint` using `theme_dark`.

use core::fmt::Write as _;

use shape_renderer_core::model::CameraState;
use shape_renderer_core::render_object::{
    RFill, RPaint, RStroke, RStrokeCap, RStrokeJoin, RText, RTextAlign, RTextMode, RTextRun,
    RTextValign, RenderObject, RenderObjectScene,
};

use crate::layout::{layout_children, origin};
use crate::quant::{js_round, q, QUANT_PER_PX};
use crate::theme::resolve_text_paint;
use crate::widget::{
    Button, Icon, Paint, RectStyle, Segment, Slider, Swatch, TextInput, TextPaint, Toggle, Widget,
};

/// Walk the tree and emit one RenderObject per primitive (Rect / Text) or per
/// composite part (Button/Swatch/Toggle/Slider/Segment/TextInput sub-parts).
/// `viewport_px` is unused for a static absolute/flex tree; it is accepted so a
/// P2 right/bottom-anchored layout can read it.
pub fn render(tree: &Widget, _viewport_px: (f64, f64), theme_dark: bool) -> RenderObjectScene {
    let mut objects: Vec<RenderObject> = Vec::new();
    // Seed with the root's own origin; from here every arm places AT its offset
    // (it never re-adds its own x/y — `layout_children` is the only position math).
    let (ox, oy) = origin(tree);
    emit(tree, ox, oy, theme_dark, &mut objects);
    RenderObjectScene {
        scene_id: "ui-scene".to_string(),
        camera: CameraState {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        },
        objects,
        selection: None,
        multi_select: Vec::new(),
    }
}

/// `off_x/off_y` is the widget's ALREADY-RESOLVED screen origin (the caller added
/// any container/cursor offset). An arm places AT (off_x, off_y) and never adds
/// its own x/y again — `layout_children` already accounted for it.
fn emit(widget: &Widget, off_x: f64, off_y: f64, theme_dark: bool, out: &mut Vec<RenderObject>) {
    match widget {
        Widget::Container(c) => {
            if c.clip {
                // A clipping container masks its descendants to its own box (the
                // renderer's stencil clip pass rasterizes this object's region). Emit
                // the clip object first (an invisible region — no fill/stroke), then
                // parent every descendant to it so they fall inside the clip subtree.
                let clip_id = c.id.clone();
                out.push(clip_object(clip_id.clone(), next_order(out.len()), off_x, off_y, c.w, c.h));
                let start = out.len();
                for (child, cx, cy) in layout_children(c) {
                    emit(child, off_x + cx, off_y + cy, theme_dark, out);
                }
                // Chain the immediate subtree roots (those still parent-less) up to the
                // clipper; deeper descendants already carry their own parent.
                for obj in &mut out[start..] {
                    if obj.parent.is_none() {
                        obj.parent = Some(clip_id.clone());
                    }
                }
            } else {
                for (child, cx, cy) in layout_children(c) {
                    emit(child, off_x + cx, off_y + cy, theme_dark, out);
                }
            }
        }
        Widget::Rect(r) => {
            out.push(rect_object(
                r.id.clone(),
                next_order(out.len()),
                off_x,
                off_y,
                r.w,
                r.h,
                &r.style,
            ));
        }
        Widget::Text(t) => {
            out.push(text_object(
                t.id.clone(),
                next_order(out.len()),
                off_x,
                off_y,
                t.w,
                t.h,
                &t.label,
                t.size_px,
                &t.color,
                t.align_center,
                theme_dark,
            ));
        }
        Widget::Icon(i) => emit_icon(i, off_x, off_y, out),
        Widget::Button(b) => emit_button(b, off_x, off_y, theme_dark, out),
        Widget::Swatch(s) => emit_swatch(s, off_x, off_y, out),
        Widget::Toggle(t) => emit_toggle(t, off_x, off_y, out),
        Widget::Slider(s) => emit_slider(s, off_x, off_y, out),
        Widget::Segment(s) => emit_segment(s, off_x, off_y, theme_dark, out),
        Widget::TextInput(t) => emit_text_input(t, off_x, off_y, theme_dark, out),
    }
}

fn emit_button(b: &Button, off_x: f64, off_y: f64, theme_dark: bool, out: &mut Vec<RenderObject>) {
    let sx = off_x;
    let sy = off_y;
    out.push(rect_object(
        b.id.clone(),
        next_order(out.len()),
        sx,
        sy,
        b.w,
        b.h,
        &b.style,
    ));
    out.push(text_object(
        format!("{}::label", b.id),
        next_order(out.len()),
        sx,
        sy,
        b.w,
        b.h,
        &b.label,
        b.label_size_px,
        &b.label_color,
        true,
        theme_dark,
    ));
}

fn emit_icon(i: &Icon, off_x: f64, off_y: f64, out: &mut Vec<RenderObject>) {
    let geometry_d = scale_icon_path(&i.d, i.w, i.h);
    let fill = i.fill.as_ref().map(|p| RFill {
        paint: paint_to_rpaint(p),
        opacity: 1.0,
    });
    // Line-art glyphs are round-cap/round-join (SF-Symbols feel).
    let stroke = i.stroke.as_ref().map(|(p, width)| RStroke {
        paint: paint_to_rpaint(p),
        width: *width,
        opacity: 1.0,
        dash: Vec::new(),
        cap: RStrokeCap::Round,
        join: RStrokeJoin::Round,
    });
    out.push(RenderObject {
        id: i.id.clone(),
        parent: None,
        order: next_order(out.len()),
        transform: translate(off_x, off_y),
        geometry_d,
        fill,
        stroke,
        text: None,
        anchors: Vec::new(),
        clip: false,
        hidden: false,
        locked: false,
    });
}

fn emit_swatch(s: &Swatch, off_x: f64, off_y: f64, out: &mut Vec<RenderObject>) {
    let style = RectStyle {
        fill: Some(s.fill.clone()),
        stroke: s
            .selected
            .then(|| (Paint::Token("selection-ring".to_string()), 2.0)),
        corner_radius: 6.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        s.id.clone(),
        next_order(out.len()),
        off_x,
        off_y,
        s.w,
        s.h,
        &style,
    ));
}

fn emit_toggle(t: &Toggle, off_x: f64, off_y: f64, out: &mut Vec<RenderObject>) {
    let sx = off_x;
    let sy = off_y;
    let track_token = if t.on {
        "selection-ring"
    } else {
        "surface-muted"
    };
    let track_style = RectStyle {
        fill: Some(Paint::Token(track_token.to_string())),
        stroke: None,
        corner_radius: t.h / 2.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        t.id.clone(),
        next_order(out.len()),
        sx,
        sy,
        t.w,
        t.h,
        &track_style,
    ));

    // Knob: a circle (corner_radius == half) inset by `pad`, slid to the on/off end.
    let pad = 2.0;
    let knob = t.h - pad * 2.0;
    let knob_x = if t.on {
        sx + t.w - knob - pad
    } else {
        sx + pad
    };
    let knob_style = RectStyle {
        fill: Some(Paint::Token("surface".to_string())),
        stroke: None,
        corner_radius: knob / 2.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        format!("{}::knob", t.id),
        next_order(out.len()),
        knob_x,
        sy + pad,
        knob,
        knob,
        &knob_style,
    ));
}

fn emit_slider(s: &Slider, off_x: f64, off_y: f64, out: &mut Vec<RenderObject>) {
    let sx = off_x;
    let sy = off_y;
    let value = s.value.clamp(0.0, 1.0);
    let track_style = RectStyle {
        fill: Some(Paint::Token("surface-muted".to_string())),
        stroke: None,
        corner_radius: s.h / 2.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        s.id.clone(),
        next_order(out.len()),
        sx,
        sy,
        s.w,
        s.h,
        &track_style,
    ));

    let fill_style = RectStyle {
        fill: Some(Paint::Token("selection-ring".to_string())),
        stroke: None,
        corner_radius: s.h / 2.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        format!("{}::fill", s.id),
        next_order(out.len()),
        sx,
        sy,
        value * s.w,
        s.h,
        &fill_style,
    ));

    // Knob: a square chip centered at `value` along the track.
    let knob = s.h;
    let knob_x = sx + value * s.w - knob / 2.0;
    let knob_style = RectStyle {
        fill: Some(Paint::Token("surface".to_string())),
        stroke: None,
        corner_radius: knob / 2.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        format!("{}::knob", s.id),
        next_order(out.len()),
        knob_x,
        sy,
        knob,
        knob,
        &knob_style,
    ));
}

fn emit_segment(
    s: &Segment,
    off_x: f64,
    off_y: f64,
    theme_dark: bool,
    out: &mut Vec<RenderObject>,
) {
    let sx = off_x;
    let sy = off_y;
    // The trough every cell sits on (inactive cells read against this shared track —
    // a segmented control has no per-cell idle pill, the track IS their background).
    let track_style = RectStyle {
        fill: Some(Paint::Token("surface-muted".to_string())),
        stroke: None,
        corner_radius: 8.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        s.id.clone(),
        next_order(out.len()),
        sx,
        sy,
        s.w,
        s.h,
        &track_style,
    ));

    let cell_count = s.labels.len().max(1);
    let cell_w = s.w / cell_count as f64;

    if s.selected < s.labels.len() {
        // The selected cell is a raised `accent-soft` pill INSET within the trough (the
        // active-control language shared with the align grid / toolbar), so it reads as
        // one selected segment rather than a full-bleed bar.
        const PILL_INSET: f64 = 2.0;
        let sel_style = RectStyle {
            fill: Some(Paint::Token("accent-soft".to_string())),
            stroke: None,
            corner_radius: 6.0,
            opacity: 1.0,
        };
        out.push(rect_object(
            format!("{}::sel", s.id),
            next_order(out.len()),
            sx + s.selected as f64 * cell_w + PILL_INSET,
            sy + PILL_INSET,
            (cell_w - PILL_INSET * 2.0).max(0.0),
            (s.h - PILL_INSET * 2.0).max(0.0),
            &sel_style,
        ));
    }

    for (n, label) in s.labels.iter().enumerate() {
        out.push(text_object(
            format!("{}::seg{n}::label", s.id),
            next_order(out.len()),
            sx + n as f64 * cell_w,
            sy,
            cell_w,
            s.h,
            label,
            s.label_size_px,
            &s.label_color,
            true,
            theme_dark,
        ));
    }
}

/// Horizontal inset for a text-input's value/placeholder (and the trailing edge
/// the caret sits off of), so glyphs are not jammed against the field border.
const TEXT_INPUT_PAD: f64 = 8.0;

fn emit_text_input(
    t: &TextInput,
    off_x: f64,
    off_y: f64,
    theme_dark: bool,
    out: &mut Vec<RenderObject>,
) {
    let sx = off_x;
    let sy = off_y;
    // Idle: a `surface-muted` field with a low-alpha `hairline` border (the macOS
    // inset-field look); focused: the `selection-ring` accent border at 2px.
    let (stroke_token, stroke_w) = if t.focused {
        ("selection-ring", 2.0)
    } else {
        ("hairline", 1.0)
    };
    let body_style = RectStyle {
        fill: Some(Paint::Token("surface-muted".to_string())),
        stroke: Some((Paint::Token(stroke_token.to_string()), stroke_w)),
        corner_radius: 6.0,
        opacity: 1.0,
    };
    out.push(rect_object(
        t.id.clone(),
        next_order(out.len()),
        sx,
        sy,
        t.w,
        t.h,
        &body_style,
    ));

    let shown = if t.value.is_empty() {
        &t.placeholder
    } else {
        &t.value
    };
    let pad = TEXT_INPUT_PAD;
    out.push(text_object(
        format!("{}::value", t.id),
        next_order(out.len()),
        sx + pad,
        sy,
        t.w - pad * 2.0,
        t.h,
        shown,
        t.size_px,
        &t.color,
        false,
        theme_dark,
    ));

    if t.focused {
        // P2 caret = end-of-text; cursor positioning is the next slice. A thin
        // rect at the value box's trailing edge.
        let caret_w = 2.0;
        let caret_x = sx + t.w - pad - caret_w;
        let caret_inset = 6.0;
        let caret_style = RectStyle {
            fill: Some(Paint::Token("selection-ring".to_string())),
            stroke: None,
            corner_radius: 0.0,
            opacity: 1.0,
        };
        out.push(rect_object(
            format!("{}::caret", t.id),
            next_order(out.len()),
            caret_x,
            sy + caret_inset,
            caret_w,
            t.h - caret_inset * 2.0,
            &caret_style,
        ));
    }
}

/// Order is a sort-key string in tree order (`a0000`, `a0001`, …), matching the
/// existing `"a0"`-style order keys.
fn next_order(idx: usize) -> String {
    format!("a{idx:04}")
}

/// A pure-translate transform placing object-local (0,0) at screen px (sx,sy).
/// Transform stays in px (NOT quantized — only geometry is quantized).
fn translate(sx: f64, sy: f64) -> [[f64; 3]; 3] {
    [[1.0, 0.0, sx], [0.0, 1.0, sy], [0.0, 0.0, 1.0]]
}

fn paint_to_rpaint(paint: &Paint) -> RPaint {
    match paint {
        Paint::Token(name) => RPaint::Token { name: name.clone() },
        Paint::Solid(color) => RPaint::Solid {
            color: color.clone(),
        },
    }
}

fn rect_object(
    id: String,
    order: String,
    sx: f64,
    sy: f64,
    w: f64,
    h: f64,
    style: &RectStyle,
) -> RenderObject {
    let geometry_d = if style.corner_radius > 0.0 {
        rounded_rect_path(w, h, style.corner_radius)
    } else {
        rect_path(w, h)
    };
    // A UI body is screen chrome the shell fully styles, so it must NEVER inherit the
    // renderer's canvas-shape structural defaults (an opaque white `default_fill` + a
    // `#283644` `default_stroke` ribbon) — those only fire when the RenderObject hands
    // over `None`. A borderless/fill-less body (the resting icon-button, menu row, and
    // action-button bodies) therefore emits an EXPLICIT fully-transparent fill, which
    // the object pipeline reads as "decorative-empty" and paints with no fill mesh, no
    // shadow, and no default-stroke ribbon — while the hover/active projection still
    // swaps in an opaque token fill. Declared paints pass through untouched.
    let fill = Some(match style.fill.as_ref() {
        Some(p) => RFill {
            paint: paint_to_rpaint(p),
            opacity: style.opacity,
        },
        None => RFill {
            paint: RPaint::Solid {
                color: "#000000".to_string(),
            },
            opacity: 0.0,
        },
    });
    let stroke = style.stroke.as_ref().map(|(p, width)| RStroke {
        paint: paint_to_rpaint(p),
        width: *width,
        opacity: 1.0,
        dash: Vec::new(),
        cap: Default::default(),
        join: Default::default(),
    });
    RenderObject {
        id,
        parent: None,
        order,
        transform: translate(sx, sy),
        geometry_d,
        fill,
        stroke,
        text: None,
        anchors: Vec::new(),
        clip: false,
        hidden: false,
        locked: false,
    }
}

/// A clip-region object: the container's box geometry carrying `clip: true` and no
/// paint, so the renderer's stencil pass masks this object's descendants to the box
/// without painting anything itself.
fn clip_object(id: String, order: String, sx: f64, sy: f64, w: f64, h: f64) -> RenderObject {
    RenderObject {
        id,
        parent: None,
        order,
        transform: translate(sx, sy),
        geometry_d: rect_path(w, h),
        fill: None,
        stroke: None,
        text: None,
        anchors: Vec::new(),
        clip: true,
        hidden: false,
        locked: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn text_object(
    id: String,
    order: String,
    sx: f64,
    sy: f64,
    w: f64,
    h: f64,
    label: &str,
    size_px: f64,
    color: &TextPaint,
    align_center: bool,
    theme_dark: bool,
) -> RenderObject {
    let align = if align_center {
        RTextAlign::Center
    } else {
        RTextAlign::Start
    };
    let resolved = resolve_text_paint(color, theme_dark);
    RenderObject {
        id,
        parent: None,
        order,
        transform: translate(sx, sy),
        // The label box: RText halign/valign resolve against this region.
        geometry_d: rect_path(w, h),
        fill: None,
        stroke: None,
        text: Some(RText {
            runs: vec![RTextRun {
                text: label.to_string(),
                color: resolved,
                // The build path DE-quantizes `size` (`/QUANT_PER_PX`), so emit the
                // wire-quantized value — mirror scene-core's `default_text_size`.
                size: size_px * QUANT_PER_PX,
                bold: false,
                italic: false,
                font: String::new(),
                // Screen-space UI chrome: device-resolution coverage, not SDF, so
                // fixed small-size text stays crisp like browser CSS text.
                mode: RTextMode::Coverage,
            }],
            align,
            valign: RTextValign::Middle,
        }),
        anchors: Vec::new(),
        clip: false,
        hidden: false,
        locked: false,
    }
}

/// Scale an SVG-subset path authored in a 24×24 box to a `w`×`h` icon box and
/// quantize. Absolute `M`/`L`/`C`/`Z` only; every command takes coordinate PAIRS
/// (`M`/`L`: 1 pair, `C`: 3 pairs, `Z`: none). A coord `c` maps `x → c/24·w`,
/// `y → c/24·h`, then `q`. Unknown commands or a short pair run are skipped so a
/// malformed glyph degrades to an empty/partial path rather than panicking.
fn scale_icon_path(d: &str, w: f64, h: f64) -> String {
    let sx = w / 24.0;
    let sy = h / 24.0;
    let mut nums = Vec::<f64>::new();
    let mut out = String::new();
    let mut tokens = d.split_whitespace().peekable();

    let map_pair = |nums: &[f64]| (q(nums[0] * sx), q(nums[1] * sy));

    while let Some(tok) = tokens.next() {
        let cmd = match tok {
            "M" | "L" | "C" | "Z" => tok,
            _ => continue,
        };
        let want = match cmd {
            "M" | "L" => 2,
            "C" => 6,
            _ => 0,
        };
        nums.clear();
        for _ in 0..want {
            match tokens.peek().and_then(|t| t.parse::<f64>().ok()) {
                Some(v) => {
                    tokens.next();
                    nums.push(v);
                }
                None => break,
            }
        }
        if nums.len() != want {
            continue;
        }
        match cmd {
            "M" | "L" => {
                let (x, y) = map_pair(&nums);
                let _ = write!(out, "{}{cmd} {x} {y}", sep(&out));
            }
            "C" => {
                let (x1, y1) = map_pair(&nums[0..2]);
                let (x2, y2) = map_pair(&nums[2..4]);
                let (x3, y3) = map_pair(&nums[4..6]);
                let _ = write!(out, "{}C {x1} {y1} {x2} {y2} {x3} {y3}", sep(&out));
            }
            "Z" => {
                let _ = write!(out, "{}Z", sep(&out));
            }
            _ => unreachable!(),
        }
    }
    out
}

/// A single space between path commands (none before the first).
fn sep(out: &str) -> &'static str {
    if out.is_empty() {
        ""
    } else {
        " "
    }
}

/// A closed rect path-string of `w`×`h` logical px (object-local), quantized.
/// Exact mirror of scene-core `primitives.rs::rect_path`.
fn rect_path(w: f64, h: f64) -> String {
    format!("M 0 0 L {} 0 L {} {} L 0 {} Z", q(w), q(w), q(h), q(h))
}

/// A closed rounded-rect path-string via four cubic corners; radius clamped to
/// `min(r, w/2, h/2)`. The kappa offset is rounded from the quantized radius
/// (mirrors `ellipse_path`'s kappa rounding). `r <= 0` ⇒ sharp rect.
fn rounded_rect_path(w: f64, h: f64, r: f64) -> String {
    if r <= 0.0 {
        return rect_path(w, h);
    }
    let r = r.min(w / 2.0).min(h / 2.0);
    let qw = q(w);
    let qh = q(h);
    let qr = q(r);
    let k = js_round(f64::from(qr) * 0.5523);
    let mut out = String::new();
    let _ = write!(out, "M {qr} 0");
    let _ = write!(out, " L {} 0", qw - qr);
    let _ = write!(out, " C {} 0 {qw} {} {qw} {qr}", qw - qr + k, qr - k);
    let _ = write!(out, " L {qw} {}", qh - qr);
    let _ = write!(
        out,
        " C {qw} {} {} {qh} {} {qh}",
        qh - qr + k,
        qw - qr + k,
        qw - qr
    );
    let _ = write!(out, " L {qr} {qh}");
    let _ = write!(out, " C {} {qh} 0 {} 0 {}", qr - k, qh - qr + k, qh - qr);
    let _ = write!(out, " L 0 {qr}");
    let _ = write!(out, " C 0 {} {} 0 {qr} 0", qr - k, qr - k);
    out.push_str(" Z");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hit::hit;
    use crate::widget::{Axis, Button, Container, CrossAlign, Edges, Icon, MainAlign, Rect, Text};

    fn proof_button() -> Widget {
        Widget::Button(Button {
            id: "ui-proof-button".to_string(),
            x: 24.0,
            y: 24.0,
            w: 140.0,
            h: 40.0,
            label: "Rust UI".to_string(),
            style: RectStyle {
                fill: Some(Paint::Token("surface".to_string())),
                stroke: Some((Paint::Token("selection-ring".to_string()), 2.0)),
                corner_radius: 12.0,
                opacity: 1.0,
            },
            label_size_px: 16.0,
            label_color: TextPaint::Hex("#ffffff".to_string()),
        })
    }

    fn absolute_container(children: Vec<Widget>) -> Container {
        Container {
            id: "panel".to_string(),
            x: 100.0,
            y: 50.0,
            w: 0.0,
            h: 0.0,
            direction: Axis::None,
            spacing: 0.0,
            main_align: MainAlign::Start,
            padding: Edges::all(0.0),
            align: CrossAlign::Start,
            clip: false,
            children,
        }
    }

    fn run_color(obj: &RenderObject) -> &str {
        &obj.text.as_ref().expect("text").runs[0].color
    }

    /// A 24×24 path scaled to a 12×12 icon box halves every coord, then quantizes
    /// (q at 8u/px). FAILS if the scale or quant drifts, or a command is dropped.
    /// `M 0 0 L 24 0 C 24 12 12 24 0 24 Z` → at half-scale: 0,0 / 12,0 / 12,6 6,12 0,12.
    #[test]
    fn scale_icon_path_scales_24box_to_icon_box_and_quantizes() {
        let scaled = scale_icon_path("M 0 0 L 24 0 C 24 12 12 24 0 24 Z", 12.0, 12.0);
        // 24px → 12px → q = 96 ; 12px → 6px → q = 48 ; 6px → q = 48? (6*8=48) etc.
        assert_eq!(scaled, "M 0 0 L 96 0 C 96 48 48 96 0 96 Z");
        // Round-trip the command structure: one M, one L, one C (3 pairs), one Z.
        assert!(scaled.starts_with("M "));
        assert!(scaled.ends_with(" Z"));
        assert_eq!(scaled.matches(" C ").count(), 1);
        // A malformed/unknown command degrades, never panics.
        assert_eq!(scale_icon_path("Q 1 2 3 4", 24.0, 24.0), "");
        // Identity box (24×24) preserves the authored coords (q at 8u/px).
        assert_eq!(
            scale_icon_path("M 3 3 L 21 21", 24.0, 24.0),
            "M 24 24 L 168 168"
        );
    }

    /// An Icon emits ONE RenderObject carrying the scaled path, a round-cap/join
    /// stroke, and the token paints. FAILS if the icon stops riding the path+stroke
    /// seam (e.g. a renderer change is required) or caps regress to butt.
    #[test]
    fn render_icon_emits_one_object_with_scaled_path_and_round_stroke() {
        let icon = Widget::Icon(Icon {
            id: "cmd:draw".to_string(),
            x: 10.0,
            y: 20.0,
            w: 24.0,
            h: 24.0,
            d: "M 4 4 L 20 20".to_string(),
            fill: None,
            stroke: Some((Paint::Token("text".to_string()), 1.6)),
        });
        let scene = render(&icon, (400.0, 400.0), false);
        assert_eq!(scene.objects.len(), 1);
        let obj = &scene.objects[0];
        assert_eq!(obj.id, "cmd:draw");
        assert_eq!(
            obj.transform,
            [[1.0, 0.0, 10.0], [0.0, 1.0, 20.0], [0.0, 0.0, 1.0]]
        );
        assert_eq!(obj.geometry_d, "M 32 32 L 160 160");
        assert!(obj.fill.is_none());
        let stroke = obj.stroke.as_ref().expect("stroke");
        assert_eq!(stroke.width, 1.6);
        assert_eq!(stroke.cap, RStrokeCap::Round);
        assert_eq!(stroke.join, RStrokeJoin::Round);
        match stroke.paint {
            RPaint::Token { ref name } => assert_eq!(name, "text"),
            _ => panic!("icon stroke must be Token(text)"),
        }
    }

    /// A `RectStyle.opacity < 1.0` reaches the emitted `RFill.opacity` (the path the
    /// translucent soft-shadow underlay depends on). FAILS if rect_object hardcodes
    /// 1.0 again. Default opacity stays 1.0 for every existing opaque fill.
    #[test]
    fn rect_opacity_threads_into_emitted_rfill() {
        let translucent = Widget::Rect(Rect {
            id: "shadow".to_string(),
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 40.0,
            style: RectStyle {
                fill: Some(Paint::Solid("#000000".to_string())),
                stroke: None,
                corner_radius: 8.0,
                opacity: 0.12,
            },
            hoverable: false,
        });
        let scene = render(&translucent, (200.0, 200.0), false);
        let fill = scene.objects[0].fill.as_ref().expect("fill");
        assert_eq!(fill.opacity, 0.12);
        // The default keeps an opaque fill at 1.0 (no regression for normal rects).
        let opaque = Widget::Rect(Rect {
            id: "r".to_string(),
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            style: RectStyle {
                fill: Some(Paint::Token("surface".to_string())),
                ..Default::default()
            },
            hoverable: false,
        });
        let opaque_scene = render(&opaque, (200.0, 200.0), false);
        assert_eq!(opaque_scene.objects[0].fill.as_ref().unwrap().opacity, 1.0);
    }

    #[test]
    fn rect_path_is_quantized_absolute() {
        assert_eq!(
            rect_path(140.0, 40.0),
            "M 0 0 L 1120 0 L 1120 320 L 0 320 Z"
        );
    }

    #[test]
    fn rounded_rect_path_emits_four_cubic_corners_and_clamps_radius() {
        let p = rounded_rect_path(140.0, 40.0, 12.0);
        assert!(p.starts_with("M "), "starts with M: {p}");
        assert!(p.ends_with(" Z"), "ends with Z: {p}");
        assert_eq!(p.matches(" C ").count(), 4, "four cubic corners: {p}");
        assert_eq!(
            rounded_rect_path(20.0, 20.0, 1000.0),
            rounded_rect_path(20.0, 20.0, 10.0)
        );
        for tok in rounded_rect_path(20.0, 20.0, 1000.0).split_whitespace() {
            if let Ok(v) = tok.parse::<i32>() {
                assert!(v >= 0, "no negative coord token: {tok}");
            }
        }
    }

    #[test]
    fn render_button_emits_body_and_label_with_translate_transform() {
        let scene = render(&proof_button(), (800.0, 600.0), false);
        assert_eq!(scene.camera.x, 0.0);
        assert_eq!(scene.camera.y, 0.0);
        assert_eq!(scene.camera.zoom, 1.0);
        assert_eq!(scene.objects.len(), 2);

        let body = &scene.objects[0];
        assert_eq!(body.id, "ui-proof-button");
        assert_eq!(
            body.transform,
            [[1.0, 0.0, 24.0], [0.0, 1.0, 24.0], [0.0, 0.0, 1.0]]
        );
        assert_eq!(body.geometry_d.matches(" C ").count(), 4);
        match body.fill.as_ref().expect("fill").paint {
            RPaint::Token { ref name } => assert_eq!(name, "surface"),
            _ => panic!("body fill must be Token(surface)"),
        }
        let stroke = body.stroke.as_ref().expect("stroke");
        assert_eq!(stroke.width, 2.0);
        match stroke.paint {
            RPaint::Token { ref name } => assert_eq!(name, "selection-ring"),
            _ => panic!("body stroke must be Token(selection-ring)"),
        }
        assert!(body.text.is_none());

        let label = &scene.objects[1];
        assert_eq!(label.id, "ui-proof-button::label");
        let text = label.text.as_ref().expect("label text");
        assert_eq!(text.runs.len(), 1);
        assert_eq!(text.runs[0].text, "Rust UI");
        assert_eq!(text.runs[0].size, 16.0 * QUANT_PER_PX);
        assert_eq!(text.runs[0].color, "#ffffff");
        assert_eq!(text.align, RTextAlign::Center);
        assert_eq!(text.valign, RTextValign::Middle);
    }

    /// FALSIFIABLE: ui-core (the screen-space UI chrome producer) emits its text in
    /// COVERAGE mode, so it takes the crisp device-resolution path instead of the SDF
    /// path canvas text keeps. Fails if the emit ever reverts to the SDF default —
    /// which would re-blur the UI text this change exists to fix.
    #[test]
    fn rendered_ui_text_is_coverage_mode() {
        let scene = render(&proof_button(), (800.0, 600.0), false);
        let label = scene
            .objects
            .iter()
            .find(|o| o.text.is_some())
            .expect("button emits a label");
        let run = &label.text.as_ref().unwrap().runs[0];
        assert_eq!(
            run.mode,
            RTextMode::Coverage,
            "UI text flips to coverage mode"
        );
    }

    #[test]
    fn rendered_label_lays_out_at_sixteen_px_through_the_build_path() {
        use shape_renderer_core::object_pipeline::build_scene_geometry_themed_with_measure;
        use shape_renderer_core::object_theme::Theme;

        let scene = render(&proof_button(), (800.0, 600.0), false);
        let unit_measure = |_ch: char, size: f32| size;
        let geo = build_scene_geometry_themed_with_measure(&scene, Theme::light(), &unit_measure);
        let second_glyph_origin_x = geo.text_vertices[6].position[0];
        let first_glyph_origin_x = geo.text_vertices[0].position[0];
        let advance = second_glyph_origin_x - first_glyph_origin_x;
        assert!(
            (advance - 16.0).abs() < 1e-4,
            "label lays out at 16px, got advance {advance}"
        );
    }

    #[test]
    fn render_container_offsets_children_in_screen_px() {
        let tree = Widget::Container(absolute_container(vec![Widget::Rect(Rect {
            id: "child".to_string(),
            x: 10.0,
            y: 5.0,
            w: 20.0,
            h: 20.0,
            style: RectStyle::default(),
            hoverable: false,
        })]));
        let scene = render(&tree, (800.0, 600.0), false);
        assert_eq!(scene.objects.len(), 1);
        assert_eq!(
            scene.objects[0].transform,
            [[1.0, 0.0, 110.0], [0.0, 1.0, 55.0], [0.0, 0.0, 1.0]]
        );
    }

    /// Theme-aware text: a token text color flips light↔dark through the build
    /// path; a Hex stays fixed. FAILS if a white label lands on a white surface
    /// (the resolved light `text` is `#1d1d1f`, never the surface `#ffffff`).
    #[test]
    fn text_paint_token_flips_with_theme_through_render() {
        let text = Widget::Text(Text {
            id: "t".to_string(),
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
            label: "hi".to_string(),
            size_px: 16.0,
            color: TextPaint::Token("text".to_string()),
            align_center: false,
        });
        let light = render(&text, (200.0, 200.0), false);
        let dark = render(&text, (200.0, 200.0), true);
        assert_eq!(run_color(&light.objects[0]), "#1d1d1f");
        assert_eq!(run_color(&dark.objects[0]), "#f5f5f7");
        assert_ne!(run_color(&light.objects[0]), "#ffffff");
    }

    /// For a flex container, the transform render() emits for child[1] equals the
    /// hit() box origin — drawn == hit, no drift. Drives REAL render() + hit().
    #[test]
    fn render_and_hit_share_laid_out_positions() {
        let tree = Widget::Container(Container {
            id: "row".to_string(),
            x: 30.0,
            y: 40.0,
            w: 200.0,
            h: 40.0,
            direction: Axis::Horizontal,
            spacing: 10.0,
            main_align: MainAlign::Start,
            padding: Edges::all(8.0),
            align: CrossAlign::Start,
            clip: false,
            children: vec![
                Widget::Rect(Rect {
                    id: "a".to_string(),
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 20.0,
                    style: RectStyle::default(),
                    hoverable: false,
                }),
                Widget::Rect(Rect {
                    id: "b".to_string(),
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 20.0,
                    style: RectStyle::default(),
                    hoverable: false,
                }),
            ],
        });
        let scene = render(&tree, (400.0, 400.0), false);
        let b = scene
            .objects
            .iter()
            .find(|o| o.id == "b")
            .expect("b object");
        let bx = b.transform[0][2];
        let by = b.transform[1][2];
        // base (30,40) + padding.l 8 + a(20) + spacing 10 = 68 ; cross padding.t 8 = 48.
        assert_eq!((bx, by), (68.0, 48.0));
        // hit at the box center lands on b (drawn == hit).
        assert_eq!(hit(&tree, (bx + 10.0, by + 10.0)), Some("b".to_string()));
    }

    #[test]
    fn slider_emits_track_fill_knob_and_hits_owner() {
        let s = Widget::Slider(Slider {
            id: "vol".to_string(),
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
            value: 0.5,
        });
        let scene = render(&s, (200.0, 200.0), false);
        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["vol", "vol::fill", "vol::knob"]);
        // filled width == 0.5*w == 50 ⇒ quantized 400.
        let filled = &scene.objects[1];
        assert!(
            filled.geometry_d.contains("400"),
            "fill width 50px: {}",
            filled.geometry_d
        );
        // hit inside lands on the owner, never a ::part.
        assert_eq!(hit(&s, (40.0, 10.0)), Some("vol".to_string()));
    }

    #[test]
    fn toggle_off_vs_on_swaps_track_token_and_knob_x() {
        let off = Widget::Toggle(Toggle {
            id: "t".to_string(),
            x: 0.0,
            y: 0.0,
            w: 48.0,
            h: 24.0,
            on: false,
        });
        let on = Widget::Toggle(Toggle {
            id: "t".to_string(),
            x: 0.0,
            y: 0.0,
            w: 48.0,
            h: 24.0,
            on: true,
        });
        let off_scene = render(&off, (200.0, 200.0), false);
        let on_scene = render(&on, (200.0, 200.0), false);
        let track_token = |o: &RenderObject| match &o.fill.as_ref().unwrap().paint {
            RPaint::Token { name } => name.clone(),
            _ => panic!("token"),
        };
        assert_eq!(track_token(&off_scene.objects[0]), "surface-muted");
        assert_eq!(track_token(&on_scene.objects[0]), "selection-ring");
        // knob x moves right when on.
        let off_knob_x = off_scene.objects[1].transform[0][2];
        let on_knob_x = on_scene.objects[1].transform[0][2];
        assert!(
            on_knob_x > off_knob_x,
            "knob slides right: {off_knob_x} -> {on_knob_x}"
        );
        assert_eq!(hit(&off, (10.0, 10.0)), Some("t".to_string()));
    }

    #[test]
    fn segment_marks_selected_cell_and_hits_owner() {
        let seg = Widget::Segment(Segment {
            id: "seg".to_string(),
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 30.0,
            labels: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            selected: 1,
            label_size_px: 14.0,
            label_color: TextPaint::Token("text".to_string()),
        });
        let scene = render(&seg, (400.0, 400.0), false);
        let sel = scene
            .objects
            .iter()
            .find(|o| o.id == "seg::sel")
            .expect("sel fill");
        // cell 1 starts at 300/3 = 100; the selected pill is inset 2px within its cell
        // (the raised-pill look), so its origin is 100 + 2 = 102.
        assert_eq!(sel.transform[0][2], 102.0);
        let labels: Vec<&str> = scene
            .objects
            .iter()
            .filter(|o| o.id.ends_with("::label"))
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(
            labels,
            ["seg::seg0::label", "seg::seg1::label", "seg::seg2::label"]
        );
        // hit any cell -> owner.
        assert_eq!(hit(&seg, (250.0, 15.0)), Some("seg".to_string()));
    }

    #[test]
    fn textinput_focus_swaps_stroke_and_emits_caret() {
        let mk = |focused: bool| {
            Widget::TextInput(TextInput {
                id: "ti".to_string(),
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 32.0,
                value: "hi".to_string(),
                focused,
                size_px: 14.0,
                color: TextPaint::Token("text".to_string()),
                placeholder: "type".to_string(),
            })
        };
        let blurred = render(&mk(false), (400.0, 400.0), false);
        let focused = render(&mk(true), (400.0, 400.0), false);
        let stroke_token = |o: &RenderObject| match &o.stroke.as_ref().unwrap().paint {
            RPaint::Token { name } => name.clone(),
            _ => panic!("token"),
        };
        assert_eq!(stroke_token(&blurred.objects[0]), "hairline");
        assert_eq!(stroke_token(&focused.objects[0]), "selection-ring");
        assert!(
            blurred.objects.iter().all(|o| !o.id.ends_with("::caret")),
            "no caret when blurred"
        );
        assert!(
            focused.objects.iter().any(|o| o.id == "ti::caret"),
            "caret when focused"
        );
        // value text is left-aligned.
        let value = focused
            .objects
            .iter()
            .find(|o| o.id == "ti::value")
            .expect("value");
        assert_eq!(value.text.as_ref().unwrap().align, RTextAlign::Start);
    }

    /// The value/placeholder text x-origin is inset from the field body x by
    /// `TEXT_INPUT_PAD`, the caret stays off the right border by the same pad, and
    /// the (now `fill: None`) value text paints NO box — so glyphs read in both
    /// themes instead of jamming the border or sitting in a white bbox. FAILS if
    /// the inset is dropped (text snaps to the border) or a fill regresses.
    #[test]
    fn textinput_value_and_placeholder_inset_from_field_body() {
        let mk = |value: &str| {
            Widget::TextInput(TextInput {
                id: "ti".to_string(),
                x: 40.0,
                y: 10.0,
                w: 200.0,
                h: 32.0,
                value: value.to_string(),
                focused: true,
                size_px: 14.0,
                color: TextPaint::Token("text".to_string()),
                placeholder: "type here".to_string(),
            })
        };
        // A LITERAL expected inset (not `TEXT_INPUT_PAD`): asserting against the
        // constant under test would stay green if the inset were zeroed — both
        // sides would move together. The literal pins the real pixel gap.
        let expect_pad = 8.0;
        for (case, value) in [("value", "hi"), ("placeholder", "")] {
            let scene = render(&mk(value), (400.0, 400.0), false);
            let body = scene.objects.iter().find(|o| o.id == "ti").expect("body");
            let text = scene
                .objects
                .iter()
                .find(|o| o.id == "ti::value")
                .expect("text");
            let body_x = body.transform[0][2];
            let text_x = text.transform[0][2];
            // The value/placeholder x-origin is the field body x plus a positive pad —
            // not jammed against the border (text_x == body_x is the live defect).
            assert!(
                text_x > body_x,
                "{case} text x must be inset, not at the field border"
            );
            assert_eq!(
                text_x,
                body_x + expect_pad,
                "{case} text x must be inset by the field padding"
            );
            // The value/placeholder carries NO fill, so no white bbox box paints under
            // the glyphs (the p1 default-fill regression) — it reads in both themes.
            assert!(text.fill.is_none(), "{case} text must not paint a fill box");
            // The caret stays clear of the right border by the same pad.
            let caret = scene
                .objects
                .iter()
                .find(|o| o.id == "ti::caret")
                .expect("caret");
            let caret_right = caret.transform[0][2] + 2.0;
            assert!(
                caret_right <= body_x + 200.0 - expect_pad + 1e-9,
                "{case} caret must stay off the right border by the field padding"
            );
        }
    }
}
