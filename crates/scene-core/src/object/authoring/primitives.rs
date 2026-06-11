//! Tier-3 — primitive geometry construction + recolor (ported from the shell).
//!
//! The shell composes a basic primitive (rectangle/ellipse/line/text/frame) into
//! an [`Object`] the core validates and applies via an `insert-object`
//! [`ObjectOp`]. This module owns that geometry-vocabulary construction (a
//! path-string `d` + inline style + a pure-translation transform placing the
//! object at a world anchor) — the SAME logic that used to live in the shell's
//! `objectPrimitives.ts`, moved into the core so the web client builds primitives
//! with the same Rust the server links (P1). It mirrors the structure of
//! [`crate::object::templates`]: a per-kind default spec, a builder that places the spec
//! at an anchor (or sizes it to a drag span), and caller-supplied id/order.
//!
//! Geometry convention (D2): each object's geometry is object-local quantized i32
//! at [`GEOMETRY_QUANTUM_PER_PX`] (8 units/px), authored from (0,0); the world
//! placement rides the `transform` translate so a later move is matrix-only (P4).
//!
//! Byte-preservation: the emitted path-string `d` must match the shell's previous
//! output exactly (locked by tests), so the cutover changes nothing visually and
//! no golden vector embedding a primitive needs re-locking. The quantization uses
//! JS `Math.round` semantics (`floor(x + 0.5)`) to match the TS it replaces.

use core::fmt::Write as _;

use crate::object::deform::is_open_class;
use crate::object::model::{
    Fill, FillRule, Geometry, Object, Paint, Stroke, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::op::{FieldEdit, ObjectOp};

/// The primitive kinds the toolbar can author (mirrors the shell `PrimitiveKindId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveKind {
    Rectangle,
    Ellipse,
    Line,
    Text,
    Frame,
}

impl PrimitiveKind {
    /// Parse the shell's lowercase kind string, or `None` if unknown.
    pub fn from_str(s: &str) -> Option<PrimitiveKind> {
        match s {
            "rectangle" => Some(PrimitiveKind::Rectangle),
            "ellipse" => Some(PrimitiveKind::Ellipse),
            "line" => Some(PrimitiveKind::Line),
            "text" => Some(PrimitiveKind::Text),
            "frame" => Some(PrimitiveKind::Frame),
            _ => None,
        }
    }
}

/// The sentinel a user color carries when "Theme default" is picked. It is NOT a
/// CSS hex — [`paint_for_color`] maps it to a [`Paint::Token`] `{ name: "text" }`
/// so the authored object follows the theme (the renderer re-resolves the "text"
/// token per theme, dark-ish in light / light-ish in dark). Every other color
/// stays a solid hex. This is the single source of truth for the sentinel; the
/// shell re-exports it so there is no second copy.
pub const THEME_DEFAULT_COLOR: &str = "token:text";

/// Map a toolbar color to its [`Paint`]: the theme-default sentinel resolves to a
/// theme token, every other value to a solid hex.
pub fn paint_for_color(color: &str) -> Paint {
    if color == THEME_DEFAULT_COLOR {
        Paint::Token { name: crate::object::theme::Token::Text.name().to_string() }
    } else {
        Paint::Solid { color: color.to_string() }
    }
}

/// Quantize logical px to object-local integer units with JS `Math.round`
/// semantics (round half toward +∞), matching the TS this replaces exactly.
/// `floor(x + 0.5)` reproduces `Math.round` for every finite value, including the
/// negative half-integers a diagonal line drag can produce.
fn q(px: f64) -> i32 {
    if px.is_nan() {
        return 0;
    }
    let units = (px * f64::from(GEOMETRY_QUANTUM_PER_PX) + 0.5).floor();
    let clamped = units.clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the value is an exact integer in range"
    )]
    let q = clamped as i32;
    q
}

// ---------------------------------------------------------------------------
// Path-string builders (object-local, quantized) — byte-match the prior TS.
// ---------------------------------------------------------------------------

/// A closed rectangle path-string of `w`×`h` logical px (object-local).
fn rect_path(w: f64, h: f64) -> String {
    format!("M 0 0 L {} 0 L {} {} L 0 {} Z", q(w), q(w), q(h), q(h))
}

/// A closed ellipse path-string of `w`×`h` logical px via four cubic arcs (kappa
/// 0.5523), matching the TS `ellipsePath` rounding (each value quantized then the
/// kappa control offset rounded from the quantized radius).
fn ellipse_path(w: f64, h: f64) -> String {
    let cx = q(w / 2.0);
    let cy = q(h / 2.0);
    let rx = q(w / 2.0);
    let ry = q(h / 2.0);
    let kx = js_round(f64::from(rx) * 0.5523);
    let ky = js_round(f64::from(ry) * 0.5523);
    let qw = q(w);
    let qh = q(h);
    let mut out = String::new();
    let _ = write!(out, "M 0 {cy}");
    let _ = write!(out, " C 0 {} {} 0 {cx} 0", cy - ky, cx - kx);
    let _ = write!(out, " C {} 0 {qw} {} {qw} {cy}", cx + kx, cy - ky);
    let _ = write!(out, " C {qw} {} {} {qh} {cx} {qh}", cy + ky, cx + kx);
    let _ = write!(out, " C {} {qh} 0 {} 0 {cy}", cx - kx, cy + ky);
    out.push_str(" Z");
    out
}

/// A horizontal open line of `len` logical px.
fn line_path(len: f64) -> String {
    format!("M 0 0 L {} 0", q(len))
}

/// JS `Math.round` for an already-product f64 (the kappa control offset).
fn js_round(x: f64) -> i32 {
    if x.is_nan() {
        return 0;
    }
    let r = (x + 0.5).floor();
    let clamped = r.clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the value is an exact integer in range"
    )]
    let v = clamped as i32;
    v
}

// ---------------------------------------------------------------------------
// Per-kind default spec.
// ---------------------------------------------------------------------------

/// The default geometry/style for a primitive kind: an object-local path-string,
/// optional fill/stroke, and the logical-px size used to center the object on an
/// anchor. Mirrors the TS `PrimitiveSpec` + `primitiveSpec` defaults.
struct PrimitiveSpec {
    d: String,
    fill: Option<Fill>,
    stroke: Option<Stroke>,
    width: f64,
    height: f64,
}

/// A solid fill at full opacity.
fn solid_fill(hex: &str) -> Fill {
    Fill { paint: Paint::Solid { color: hex.to_string() }, opacity: 1.0 }
}

/// A solid stroke of `width_px` logical px (quantized), default round-less caps —
/// the TS specs carried only paint + width, so cap/join/dash stay at their defaults.
fn solid_stroke(hex: &str, width_px: f64) -> Stroke {
    Stroke {
        paint: Paint::Solid { color: hex.to_string() },
        width: q(width_px),
        opacity: 1.0,
        dash: Vec::new(),
        cap: crate::object::model::LineCap::Butt,
        join: crate::object::model::LineJoin::Miter,
    }
}

fn primitive_spec(kind: PrimitiveKind) -> PrimitiveSpec {
    match kind {
        PrimitiveKind::Rectangle => PrimitiveSpec {
            d: rect_path(160.0, 100.0),
            fill: Some(solid_fill("#e8eefc")),
            stroke: Some(solid_stroke("#2f7ee6", 1.5)),
            width: 160.0,
            height: 100.0,
        },
        PrimitiveKind::Ellipse => PrimitiveSpec {
            d: ellipse_path(140.0, 140.0),
            fill: Some(solid_fill("#e8eefc")),
            stroke: Some(solid_stroke("#2f7ee6", 1.5)),
            width: 140.0,
            height: 140.0,
        },
        PrimitiveKind::Line => PrimitiveSpec {
            d: line_path(200.0),
            fill: None,
            stroke: Some(solid_stroke("#5b6472", 2.0)),
            width: 200.0,
            height: 0.0,
        },
        // The text primitive is a borderless, style-less rect — no border, no
        // fill, no default text. Every object can hold text; the text "shape" is
        // one with no border that enters inline edit immediately on create.
        PrimitiveKind::Text => PrimitiveSpec {
            d: rect_path(180.0, 80.0),
            fill: None,
            stroke: None,
            width: 180.0,
            height: 80.0,
        },
        PrimitiveKind::Frame => PrimitiveSpec {
            d: rect_path(420.0, 300.0),
            fill: None,
            stroke: Some(solid_stroke("#94a3b8", 1.0)),
            width: 420.0,
            height: 300.0,
        },
    }
}

/// Override a spec's existing fill/stroke paint with the toolbar color (only the
/// paint changes; width/opacity stay). An absent field stays absent (a line has
/// no fill, the text primitive neither). `None` leaves the hardcoded defaults.
fn recolor_spec(mut spec: PrimitiveSpec, color: Option<&str>) -> PrimitiveSpec {
    if let Some(color) = color {
        let paint = paint_for_color(color);
        if let Some(fill) = spec.fill.as_mut() {
            fill.paint = paint.clone();
        }
        if let Some(stroke) = spec.stroke.as_mut() {
            stroke.paint = paint;
        }
    }
    spec
}

/// Build an [`Object`] from a spec's `d`/style at `transform`, with `id`/`order`.
/// `clip` is set for the frame primitive (D18). The geometry carries the spec's
/// path-string (parsed so the object's runtime contours are populated) and the
/// `nonZero` fill rule the shell authored.
fn object_from_spec(spec: PrimitiveSpec, id: &str, order: &str, transform: Transform3x3, clip: bool) -> Object {
    let mut geometry = Geometry { path_string: spec.d, fill_rule: FillRule::NonZero, subpaths: Vec::new() };
    // Hydrate the runtime contours from the canonical path-string; the wire form
    // (`d`) is untouched, so byte-output is exactly the authored string.
    let _ = geometry.parse();
    let mut obj = Object::new(id.to_string(), order.to_string(), geometry);
    obj.transform = transform;
    obj.fill = spec.fill;
    obj.stroke = spec.stroke;
    if clip {
        obj.clip = Some(true);
    }
    obj
}

// ---------------------------------------------------------------------------
// Public builders.
// ---------------------------------------------------------------------------

/// Build the [`Object`] for a primitive `kind`, centered on the world anchor
/// `(anchor_x, anchor_y)`, recolored to `color` (or the kind default when `color`
/// is `None`). The geometry is object-local; the world position rides a
/// pure-translation transform (D7) so a later move is matrix-only (P4).
pub fn build_primitive(
    kind: PrimitiveKind,
    anchor_x: f64,
    anchor_y: f64,
    color: Option<&str>,
    id: &str,
    order: &str,
) -> Object {
    let spec = recolor_spec(primitive_spec(kind), color);
    let tx = anchor_x - spec.width / 2.0;
    let ty = anchor_y - spec.height / 2.0;
    object_from_spec(spec, id, order, Transform3x3::translate(tx, ty), kind == PrimitiveKind::Frame)
}

/// A drag span — the gesture's start corner and current/end corner (world px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragSpan {
    pub start_x: f64,
    pub start_y: f64,
    pub end_x: f64,
    pub end_y: f64,
}

/// Build the [`Object`] for a primitive `kind` sized to a drag `span`, recolored
/// to `color` (or the kind default when `None`). Closed primitives size to the
/// normalized bbox; the open line runs corner-to-corner so a diagonal drag draws
/// a diagonal. Geometry object-local; world position on a translate (P4).
pub fn build_primitive_from_drag(
    kind: PrimitiveKind,
    span: DragSpan,
    color: Option<&str>,
    id: &str,
    order: &str,
) -> Object {
    let spec = recolor_spec(primitive_spec(kind), color);
    let (d, tx, ty) = drag_geometry(kind, span);
    let spec = PrimitiveSpec { d, ..spec };
    object_from_spec(spec, id, order, Transform3x3::translate(tx, ty), kind == PrimitiveKind::Frame)
}

/// Object-local geometry + the world translation for a primitive sized to a drag.
/// The line rides corner-to-corner (object-local from (0,0) to the end delta,
/// translated at the start point); closed kinds size to the normalized bbox and
/// translate to its min corner.
fn drag_geometry(kind: PrimitiveKind, span: DragSpan) -> (String, f64, f64) {
    if kind == PrimitiveKind::Line {
        let dx = span.end_x - span.start_x;
        let dy = span.end_y - span.start_y;
        return (format!("M 0 0 L {} {}", q(dx), q(dy)), span.start_x, span.start_y);
    }
    let min_x = span.start_x.min(span.end_x);
    let min_y = span.start_y.min(span.end_y);
    let w = (span.end_x - span.start_x).abs();
    let h = (span.end_y - span.start_y).abs();
    let d = if kind == PrimitiveKind::Ellipse { ellipse_path(w, h) } else { rect_path(w, h) };
    (d, min_x, min_y)
}

/// Author a `set-style` op recoloring `object` to `color`. Recolor only the
/// style fields the object already carries — a filled shape keeps its stroke, a
/// stroke-only line keeps being stroke-only — so a recolor never adds a paint the
/// object did not have. An object with NEITHER fill nor stroke (the borderless
/// text primitive) gets a fill so the recolor is still visible. The inverse (the
/// old style) comes from the core apply path, keeping undo correct (D21).
///
/// Anchor-semantics v3 §1: an OPEN-CLASS object (one open subpath) carries no
/// fill, so the color routes to its STROKE — recoloring it, or authoring the
/// line-default stroke when missing — and a legacy fill is left untouched (the
/// renderer skips open-class fills; the shell stays class-ignorant).
pub fn build_set_style_op(object: &Object, color: &str) -> ObjectOp {
    let paint = paint_for_color(color);
    if is_open_class(&object.geometry) {
        let mut stroke = object.stroke.clone().unwrap_or_else(|| solid_stroke("#5b6472", 2.0));
        stroke.paint = paint;
        return ObjectOp::SetStyle {
            id: object.id.clone(),
            fill: None,
            stroke: Some(FieldEdit::Set { value: stroke }),
        };
    }
    let mut fill_edit: Option<FieldEdit<Fill>> = None;
    let mut stroke_edit: Option<FieldEdit<Stroke>> = None;
    if let Some(fill) = &object.fill {
        let mut next = fill.clone();
        next.paint = paint.clone();
        fill_edit = Some(FieldEdit::Set { value: next });
    }
    if let Some(stroke) = &object.stroke {
        let mut next = stroke.clone();
        next.paint = paint.clone();
        stroke_edit = Some(FieldEdit::Set { value: next });
    }
    if object.fill.is_none() && object.stroke.is_none() {
        fill_edit = Some(FieldEdit::Set { value: Fill { paint, opacity: 1.0 } });
    }
    ObjectOp::SetStyle { id: object.id.clone(), fill: fill_edit, stroke: stroke_edit }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d_of(obj: &Object) -> &str {
        obj.geometry.path_string.as_str()
    }

    // -- d-string byte-match (lock the kappa + quantization) -----------------

    #[test]
    fn rectangle_default_d_byte_matches_prior_ts() {
        let o = build_primitive(PrimitiveKind::Rectangle, 0.0, 0.0, None, "r", "a0");
        // q(160)=1280, q(100)=800.
        assert_eq!(d_of(&o), "M 0 0 L 1280 0 L 1280 800 L 0 800 Z");
    }

    #[test]
    fn ellipse_default_d_byte_matches_prior_ts() {
        let o = build_primitive(PrimitiveKind::Ellipse, 0.0, 0.0, None, "e", "a0");
        // cx=cy=rx=ry=q(70)=560; kx=ky=round(560*0.5523)=round(309.288)=309.
        assert_eq!(
            d_of(&o),
            "M 0 560 C 0 251 251 0 560 0 C 869 0 1120 251 1120 560 C 1120 869 869 1120 560 1120 C 251 1120 0 869 0 560 Z"
        );
    }

    #[test]
    fn line_default_d_byte_matches_prior_ts() {
        let o = build_primitive(PrimitiveKind::Line, 0.0, 0.0, None, "l", "a0");
        // q(200)=1600.
        assert_eq!(d_of(&o), "M 0 0 L 1600 0");
    }

    #[test]
    fn text_and_frame_default_d_byte_match_prior_ts() {
        let t = build_primitive(PrimitiveKind::Text, 0.0, 0.0, None, "t", "a0");
        assert_eq!(d_of(&t), "M 0 0 L 1440 0 L 1440 640 L 0 640 Z"); // q(180)=1440,q(80)=640
        let f = build_primitive(PrimitiveKind::Frame, 0.0, 0.0, None, "f", "a0");
        assert_eq!(d_of(&f), "M 0 0 L 3360 0 L 3360 2400 L 0 2400 Z"); // q(420)=3360,q(300)=2400
    }

    // -- anchor centering + transform ---------------------------------------

    #[test]
    fn primitive_is_centered_on_the_anchor_via_translate() {
        let o = build_primitive(PrimitiveKind::Rectangle, 100.0, 50.0, None, "r", "a0");
        // 160x100 centered on (100,50): translate to (100-80, 50-50)=(20,0).
        assert_eq!(o.transform.m[0][2], 20.0);
        assert_eq!(o.transform.m[1][2], 0.0);
    }

    // -- defaults + recolor --------------------------------------------------

    #[test]
    fn default_rectangle_carries_the_hardcoded_fill_and_stroke() {
        let o = build_primitive(PrimitiveKind::Rectangle, 0.0, 0.0, None, "r", "a0");
        assert_eq!(o.fill.as_ref().unwrap().paint, Paint::Solid { color: "#e8eefc".into() });
        assert_eq!(o.stroke.as_ref().unwrap().paint, Paint::Solid { color: "#2f7ee6".into() });
        // q(1.5) stroke width = 12.
        assert_eq!(o.stroke.as_ref().unwrap().width, 12);
        assert_eq!(o.geometry.fill_rule, FillRule::NonZero);
    }

    #[test]
    fn color_recolors_fill_and_stroke_without_touching_width() {
        let def = build_primitive(PrimitiveKind::Rectangle, 0.0, 0.0, None, "r", "a0");
        let o = build_primitive(PrimitiveKind::Rectangle, 0.0, 0.0, Some("#abcdef"), "r2", "a0");
        assert_eq!(o.fill.as_ref().unwrap().paint, Paint::Solid { color: "#abcdef".into() });
        assert_eq!(o.stroke.as_ref().unwrap().paint, Paint::Solid { color: "#abcdef".into() });
        assert_eq!(o.stroke.as_ref().unwrap().width, def.stroke.as_ref().unwrap().width);
    }

    #[test]
    fn line_has_no_fill_only_a_recolored_stroke() {
        let o = build_primitive(PrimitiveKind::Line, 0.0, 0.0, Some("#abcdef"), "l", "a0");
        assert!(o.fill.is_none());
        assert_eq!(o.stroke.as_ref().unwrap().paint, Paint::Solid { color: "#abcdef".into() });
    }

    #[test]
    fn theme_default_color_resolves_to_the_text_token() {
        assert_eq!(paint_for_color(THEME_DEFAULT_COLOR), Paint::Token { name: "text".into() });
        assert_eq!(paint_for_color("#ef4444"), Paint::Solid { color: "#ef4444".into() });
        let o = build_primitive(PrimitiveKind::Rectangle, 0.0, 0.0, Some(THEME_DEFAULT_COLOR), "r", "a0");
        assert_eq!(o.fill.as_ref().unwrap().paint, Paint::Token { name: "text".into() });
        assert_eq!(o.stroke.as_ref().unwrap().paint, Paint::Token { name: "text".into() });
    }

    #[test]
    fn frame_primitive_sets_clip() {
        let o = build_primitive(PrimitiveKind::Frame, 0.0, 0.0, None, "f", "a0");
        assert_eq!(o.clip, Some(true));
        let r = build_primitive(PrimitiveKind::Rectangle, 0.0, 0.0, None, "r", "a0");
        assert_eq!(r.clip, None);
    }

    // -- drag sizing ---------------------------------------------------------

    #[test]
    fn drag_rectangle_sizes_to_normalized_bbox() {
        // Drag from (10,10) to (5,40): normalized bbox is 5 wide, 30 tall, min (5,10).
        let span = DragSpan { start_x: 10.0, start_y: 10.0, end_x: 5.0, end_y: 40.0 };
        let o = build_primitive_from_drag(PrimitiveKind::Rectangle, span, None, "r", "a0");
        assert_eq!(d_of(&o), "M 0 0 L 40 0 L 40 240 L 0 240 Z"); // q(5)=40, q(30)=240
        assert_eq!(o.transform.m[0][2], 5.0);
        assert_eq!(o.transform.m[1][2], 10.0);
    }

    #[test]
    fn drag_line_runs_corner_to_corner() {
        // A diagonal line from (10,10) to (40,30): local (0,0)->(30,20), at (10,10).
        let span = DragSpan { start_x: 10.0, start_y: 10.0, end_x: 40.0, end_y: 30.0 };
        let o = build_primitive_from_drag(PrimitiveKind::Line, span, None, "l", "a0");
        assert_eq!(d_of(&o), "M 0 0 L 240 160"); // q(30)=240, q(20)=160
        assert_eq!(o.transform.m[0][2], 10.0);
        assert_eq!(o.transform.m[1][2], 10.0);
    }

    #[test]
    fn drag_line_negative_delta_quantizes_with_js_round() {
        // A delta with a .5*px half-integer exercises JS round-half-up on negatives:
        // dx = -0.0625 px -> -0.5 units -> Math.round(-0.5) = 0 (not -1).
        let span = DragSpan { start_x: 0.0, start_y: 0.0, end_x: -0.0625, end_y: 0.0 };
        let o = build_primitive_from_drag(PrimitiveKind::Line, span, None, "l", "a0");
        assert_eq!(d_of(&o), "M 0 0 L 0 0");
    }

    #[test]
    fn drag_ellipse_uses_the_ellipse_path() {
        let span = DragSpan { start_x: 0.0, start_y: 0.0, end_x: 140.0, end_y: 140.0 };
        let o = build_primitive_from_drag(PrimitiveKind::Ellipse, span, None, "e", "a0");
        // Same as the default ellipse (140x140 bbox).
        assert_eq!(
            d_of(&o),
            "M 0 560 C 0 251 251 0 560 0 C 869 0 1120 251 1120 560 C 1120 869 869 1120 560 1120 C 251 1120 0 869 0 560 Z"
        );
    }

    // -- set-style recolor ---------------------------------------------------

    fn obj_with(geometry_d: &str, fill: Option<Fill>, stroke: Option<Stroke>) -> Object {
        let mut o = Object::new(
            "o1",
            "a0",
            Geometry { path_string: geometry_d.to_string(), fill_rule: FillRule::NonZero, subpaths: Vec::new() },
        );
        o.fill = fill;
        o.stroke = stroke;
        o
    }

    #[test]
    fn set_style_recolors_both_fill_and_stroke_of_a_filled_shape() {
        // Closed d: a fill-bearing shape is closed-class by definition (v3 §1).
        let o = obj_with(
            "M 0 0 L 8 0 L 8 8 Z",
            Some(solid_fill("#000000")),
            Some(solid_stroke("#111111", 2.0)),
        );
        let op = build_set_style_op(&o, "#abcdef");
        match op {
            ObjectOp::SetStyle { id, fill, stroke } => {
                assert_eq!(id, "o1");
                let FieldEdit::Set { value: f } = fill.unwrap() else { panic!("fill set") };
                assert_eq!(f.paint, Paint::Solid { color: "#abcdef".into() });
                let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
                assert_eq!(s.paint, Paint::Solid { color: "#abcdef".into() });
                assert_eq!(s.width, q(2.0)); // width preserved
            }
            other => panic!("expected set-style, got {}", other.kind()),
        }
    }

    #[test]
    fn set_style_recolors_only_the_stroke_for_a_stroke_only_object() {
        let o = obj_with("M 0 0 L 8 0", None, Some(solid_stroke("#111111", 2.0)));
        let op = build_set_style_op(&o, "#abcdef");
        let ObjectOp::SetStyle { fill, stroke, .. } = op else { panic!("set-style") };
        assert!(fill.is_none(), "no fill added to a stroke-only object");
        assert!(stroke.is_some());
    }

    #[test]
    fn set_style_gives_a_styleless_object_a_fill() {
        // Closed d: the borderless text primitive is a closed rect (v3 §1 keeps
        // the fill fallback for closed-class only).
        let o = obj_with("M 0 0 L 8 0 L 8 8 Z", None, None);
        let op = build_set_style_op(&o, "#abcdef");
        let ObjectOp::SetStyle { fill, stroke, .. } = op else { panic!("set-style") };
        assert!(stroke.is_none());
        let FieldEdit::Set { value: f } = fill.unwrap() else { panic!("fill set") };
        assert_eq!(f.paint, Paint::Solid { color: "#abcdef".into() });
        assert_eq!(f.opacity, 1.0);
    }

    // -- v3 §1: open-class color routes to the stroke, never the fill ---------

    #[test]
    fn set_style_routes_open_class_color_to_stroke_never_fill() {
        // A legacy open path carrying a fill: the recolor must not touch it.
        let o = obj_with(
            "M 0 0 L 8 0",
            Some(solid_fill("#000000")),
            Some(solid_stroke("#111111", 2.0)),
        );
        let op = build_set_style_op(&o, "#abcdef");
        let ObjectOp::SetStyle { fill, stroke, .. } = op else { panic!("set-style") };
        assert!(fill.is_none(), "open-class recolor must not author a fill edit");
        let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
        assert_eq!(s.paint, Paint::Solid { color: "#abcdef".into() });
        assert_eq!(s.width, q(2.0));
    }

    #[test]
    fn set_style_gives_a_strokeless_open_path_a_stroke_not_a_fill() {
        let o = obj_with("M 0 0 L 8 0", None, None);
        let op = build_set_style_op(&o, "#abcdef");
        let ObjectOp::SetStyle { fill, stroke, .. } = op else { panic!("set-style") };
        assert!(fill.is_none(), "open-class never gains a fill");
        let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
        assert_eq!(s.paint, Paint::Solid { color: "#abcdef".into() });
        // The line-default stroke width (q(2px)) so the recolor is visible.
        assert_eq!(s.width, q(2.0));
    }

    #[test]
    fn set_style_to_theme_default_uses_the_text_token() {
        let o = obj_with("M 0 0 L 8 0", None, Some(solid_stroke("#111111", 2.0)));
        let op = build_set_style_op(&o, THEME_DEFAULT_COLOR);
        let ObjectOp::SetStyle { stroke, .. } = op else { panic!("set-style") };
        let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
        assert_eq!(s.paint, Paint::Token { name: "text".into() });
    }

    // -- live-preview parity: the committed core object equals what a shell-side
    //    preview would draw at the same span (same d for the same span). --------

    #[test]
    fn committed_drag_object_d_matches_a_repeat_build_at_the_same_span() {
        let span = DragSpan { start_x: 3.0, start_y: 7.0, end_x: 91.0, end_y: 44.0 };
        let preview = build_primitive_from_drag(PrimitiveKind::Rectangle, span, None, "preview", "a0");
        let committed = build_primitive_from_drag(PrimitiveKind::Rectangle, span, Some("#abcdef"), "real", "a1");
        // Geometry (the visual shape) is identical regardless of id/order/color.
        assert_eq!(d_of(&preview), d_of(&committed));
        assert_eq!(preview.transform, committed.transform);
    }
}
