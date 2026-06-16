//! Lower one inspector-panel property edit to its [`ObjectOp`]. The single source
//! of truth for the per-field op synthesis the shell used to assemble in TS:
//! patching the edited field on the object's current value, choosing the op kind,
//! and — critically — owning every style/layout/sizing/text DEFAULT a borderless or
//! layout-less object gains. The shell forwards `(control_id, object, value)` and
//! authors the returned op; it makes no styling decision.
//!
//! Scope: every catalog control whose op is built from the object's current value +
//! the typed edit value. The transform/resize fields (x/y/rotation/width/height) are
//! NOT here — they route through [`set_transform_field`](super::affine::set_transform_field)
//! / [`resize_axis`](crate::object::inspector::resize_axis), which need the matrix /
//! geometry as separate inputs. An unrecognized control id (those, or a typo) is
//! `None`, so the shell keeps its transform routing.
//!
//! Pure (binding tier): no time/rng/IO. The px-denominated edits re-quantize through
//! [`quantize_units`](super::affine::quantize_units), so the shell owns neither the
//! rounding mode nor the geometry quantum.

use serde_json::Value;

use super::affine::quantize_units;
use crate::object::authoring::primitives::default_inspector_stroke;
use crate::object::model::{
    Align, AxisSizing, Fill, Layout, LayoutAxis, Lanes, Object, Paint, Sizing, Stroke, TextAlign,
    TextRun,
};
use crate::object::op::{FieldEdit, ObjectOp};

/// The default [`Layout`] a free container gains when first switched to Flow:
/// horizontal, single lane, no gap, start/start. Owned here (not the shell) so the
/// flow-default is a canvas decision the core makes. Re-clicking an already-flow
/// container preserves its current layout instead (handled in [`inspector_edit_op`]).
fn default_flow_layout() -> Layout {
    Layout {
        axis: LayoutAxis::Horizontal,
        lanes: Lanes::Count { value: 1 },
        spacing: 0,
        align: Align {
            main: crate::object::model::MainAlign::Start,
            cross: crate::object::model::CrossAlign::Start,
        },
    }
}

/// Lower one inspector control edit on `object` to its [`ObjectOp`], patching ONLY
/// the edited field on the object's CURRENT value. `value` is the panel's edit value
/// as the view handed it down (its JSON shape per control matches `inspector_view`'s
/// read value); `unit_scale` is the control's `unit_scale`, so a px-denominated edit
/// re-quantizes through the core. Returns `None` for a control this helper does not
/// own (the transform/resize fields) or one whose edit cannot apply (a layout field
/// on a layout-less object, a text field on a text-less object).
pub fn inspector_edit_op(
    object: &Object,
    control_id: &str,
    value: &Value,
    unit_scale: f64,
) -> Option<ObjectOp> {
    let id = object.id.clone();
    match control_id {
        "name" => Some(ObjectOp::SetMeta {
            id,
            name: Some(match value.as_str() {
                Some(s) if !s.is_empty() => FieldEdit::Set { value: s.to_string() },
                _ => FieldEdit::Clear,
            }),
            hidden: None,
            locked: None,
        }),
        // The toggle shows the stored `hidden` flag inverted: visible=true => hidden=false.
        "visible" => Some(ObjectOp::SetMeta {
            id,
            name: None,
            hidden: Some(!value.as_bool()?),
            locked: None,
        }),
        "locked" => Some(ObjectOp::SetMeta {
            id,
            name: None,
            hidden: None,
            locked: Some(value.as_bool()?),
        }),
        "sizing-w" | "sizing-h" => Some(sizing_op(id, object, control_id, value, unit_scale)?),
        "layout-mode" => layout_mode_op(id, object, value),
        "axis" | "lanes" | "spacing" | "align" => {
            layout_field_op(id, object, control_id, value, unit_scale)
        }
        "clip" => Some(ObjectOp::SetClip {
            id,
            clip: Some(value.as_bool()?),
        }),
        "fill" => {
            let paint: Paint = serde_json::from_value(value.clone()).ok()?;
            Some(ObjectOp::SetStyle {
                id,
                fill: Some(FieldEdit::Set { value: paint_to_fill(object.fill.as_ref(), paint) }),
                stroke: None,
            })
        }
        "stroke" => {
            let paint: Paint = serde_json::from_value(value.clone()).ok()?;
            Some(ObjectOp::SetStyle {
                id,
                fill: None,
                stroke: Some(FieldEdit::Set { value: paint_to_stroke(object.stroke.as_ref(), paint) }),
            })
        }
        "stroke-width" => Some(stroke_width_op(id, object, value, unit_scale)?),
        "font" | "font-size" | "font-weight" | "text-align" | "text-color" => {
            text_field_op(id, object, control_id, value, unit_scale)
        }
        // The transform/resize fields route through their own core surfaces; an
        // unrecognized id is a no-op.
        _ => None,
    }
}

/// Patch one axis of the sizing (default both Hug), author the full [`Sizing`]. A
/// segment click hands an [`AxisSizing`] tag; the Fixed companion px input hands a
/// bare number, re-quantized here (px * unit_scale) and wrapped as the Fixed value.
fn sizing_op(
    id: String,
    object: &Object,
    control_id: &str,
    value: &Value,
    unit_scale: f64,
) -> Option<ObjectOp> {
    let axis_sizing = match value.as_f64() {
        Some(px) => AxisSizing::Fixed { value: quantize_units(px, unit_scale) },
        None => serde_json::from_value(value.clone()).ok()?,
    };
    let current = object.sizing.unwrap_or(Sizing {
        w: AxisSizing::Hug,
        h: AxisSizing::Hug,
    });
    let sizing = if control_id == "sizing-w" {
        Sizing { w: axis_sizing, h: current.h }
    } else {
        Sizing { w: current.w, h: axis_sizing }
    };
    Some(ObjectOp::SetSizing { id, sizing: Some(sizing) })
}

/// Flow with a layout already present preserves it (re-clicking the active "Flow" is
/// a no-op, not a destructive reset to the default); only a free container
/// synthesizes the default Flow layout. "free" clears the layout.
fn layout_mode_op(id: String, object: &Object, value: &Value) -> Option<ObjectOp> {
    match value.as_str()? {
        "flow" => {
            if object.layout.is_some() {
                None
            } else {
                Some(ObjectOp::SetLayout { id, layout: Some(default_flow_layout()) })
            }
        }
        _ => Some(ObjectOp::SetLayout { id, layout: None }),
    }
}

/// Patch one layout field on the object's CURRENT layout (set-layout replaces the
/// WHOLE Layout, so a full Layout is authored). A free object with no layout cannot
/// edit a flow field; `None` then.
fn layout_field_op(
    id: String,
    object: &Object,
    control_id: &str,
    value: &Value,
    unit_scale: f64,
) -> Option<ObjectOp> {
    let mut layout = object.layout?;
    match control_id {
        "axis" => layout.axis = serde_json::from_value(value.clone()).ok()?,
        "lanes" => layout.lanes = serde_json::from_value(value.clone()).ok()?,
        // The view emits spacing in px; re-quantize (px * scale) and floor at 0.
        "spacing" => layout.spacing = quantize_units(value.as_f64()?, unit_scale).max(0),
        "align" => layout.align = serde_json::from_value::<Align>(value.clone()).ok()?,
        _ => return None,
    }
    Some(ObjectOp::SetLayout { id, layout: Some(layout) })
}

/// Edit the current stroke's width (set-style stroke). The view emits px; re-quantize
/// (px * scale) and floor at 0. A borderless object gains the core's default inspector
/// stroke (the SAME stroke `build_set_style_op` would give a borderless open path), so
/// the shell never invents a stroke color/attributes.
fn stroke_width_op(id: String, object: &Object, value: &Value, unit_scale: f64) -> Option<ObjectOp> {
    let width = quantize_units(value.as_f64()?, unit_scale).max(0);
    let stroke = match &object.stroke {
        Some(stroke) => Stroke { width, ..stroke.clone() },
        None => Stroke { width, ..default_inspector_stroke() },
    };
    Some(ObjectOp::SetStyle {
        id,
        fill: None,
        stroke: Some(FieldEdit::Set { value: stroke }),
    })
}

/// Patch one text field on the object's CURRENT text (font/size/bold/color on
/// runs[0]; align on Text), author the full [`Text`] through set-text. `None` when
/// the object carries no text. size re-quantizes (px * scale) and floors at 1.
fn text_field_op(
    id: String,
    object: &Object,
    control_id: &str,
    value: &Value,
    unit_scale: f64,
) -> Option<ObjectOp> {
    let mut text = object.text.clone()?;
    if text.runs.is_empty() {
        text.runs.push(TextRun {
            text: String::new(),
            color: None,
            size: None,
            bold: false,
            italic: false,
            font: None,
        });
    }
    let run = &mut text.runs[0];
    match control_id {
        "font" => run.font = value.as_str().map(str::to_string),
        "font-size" => run.size = Some(quantize_units(value.as_f64()?, unit_scale).max(1)),
        "font-weight" => run.bold = value.as_str() == Some("bold"),
        "text-color" => {
            let paint: Paint = serde_json::from_value(value.clone()).ok()?;
            run.color = match paint {
                Paint::Solid { color } => Some(color),
                _ => None,
            };
        }
        "text-align" => text.align = serde_json::from_value::<TextAlign>(value.clone()).ok()?,
        _ => return None,
    }
    Some(ObjectOp::SetText { id, text: Some(text) })
}

/// Build the [`Fill`] to set: keep the object's current opacity (default 1) and swap
/// only the paint.
fn paint_to_fill(current: Option<&Fill>, paint: Paint) -> Fill {
    Fill {
        paint,
        opacity: current.map_or(1.0, |f| f.opacity),
    }
}

/// Build the [`Stroke`] to set: keep the object's current attributes (or the core's
/// default inspector stroke for a borderless object) and swap only the paint.
fn paint_to_stroke(current: Option<&Stroke>, paint: Paint) -> Stroke {
    match current {
        Some(stroke) => Stroke { paint, ..stroke.clone() },
        None => Stroke { paint, ..default_inspector_stroke() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, Text, GEOMETRY_QUANTUM_PER_PX};

    const Q: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

    fn obj(d: &str) -> Object {
        Object::new(
            "o1",
            "a0",
            Geometry { path_string: d.to_string(), fill_rule: FillRule::NonZero, subpaths: Vec::new() },
        )
    }

    fn closed() -> Object {
        obj("M 0 0 L 8 0 L 8 8 Z")
    }

    fn edit(object: &Object, control_id: &str, value: Value, unit_scale: f64) -> Option<ObjectOp> {
        inspector_edit_op(object, control_id, &value, unit_scale)
    }

    #[test]
    fn name_sets_or_clears() {
        assert_eq!(
            edit(&closed(), "name", Value::String("Hero".into()), 1.0),
            Some(ObjectOp::SetMeta {
                id: "o1".into(),
                name: Some(FieldEdit::Set { value: "Hero".into() }),
                hidden: None,
                locked: None,
            })
        );
        assert_eq!(
            edit(&closed(), "name", Value::String(String::new()), 1.0),
            Some(ObjectOp::SetMeta {
                id: "o1".into(),
                name: Some(FieldEdit::Clear),
                hidden: None,
                locked: None,
            })
        );
    }

    #[test]
    fn visible_inverts_to_hidden() {
        let ObjectOp::SetMeta { hidden, .. } = edit(&closed(), "visible", Value::Bool(false), 1.0).unwrap()
        else {
            panic!("set-meta")
        };
        assert_eq!(hidden, Some(true));
    }

    #[test]
    fn fill_keeps_opacity_swaps_paint() {
        let mut o = closed();
        o.fill = Some(Fill { paint: Paint::Solid { color: "#ff0000".into() }, opacity: 0.5 });
        let op = edit(&o, "fill", serde_json::json!({ "kind": "solid", "color": "#00ff00" }), 1.0).unwrap();
        let ObjectOp::SetStyle { fill, stroke, .. } = op else { panic!("set-style") };
        assert!(stroke.is_none(), "fill edit touches only fill");
        let FieldEdit::Set { value: f } = fill.unwrap() else { panic!("fill set") };
        assert_eq!(f.paint, Paint::Solid { color: "#00ff00".into() });
        assert_eq!(f.opacity, 0.5, "current opacity preserved");
    }

    // The borderless object's gained stroke is the core's default inspector stroke —
    // never a shell-invented color/attribute set. It MUST equal what build_set_style_op
    // gives a borderless open path (the same default-stroke token convention).
    #[test]
    fn borderless_stroke_width_gains_the_core_default_not_an_invented_one() {
        let op = edit(&closed(), "stroke-width", serde_json::json!(6.0), Q).unwrap();
        let ObjectOp::SetStyle { stroke, .. } = op else { panic!("set-style") };
        let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
        // Width re-quantized px * Q (6 px * 8 = 48), the rest the core default stroke.
        assert_eq!(s.width, 48);
        let core_default = default_inspector_stroke();
        assert_eq!(s.paint, core_default.paint, "default stroke paint is the core's");
        assert_eq!(s.opacity, core_default.opacity);
        assert_eq!(s.cap, core_default.cap);
        assert_eq!(s.join, core_default.join);
        assert_eq!(s.dash, core_default.dash, "dash carried (not omitted to a wire default)");
    }

    #[test]
    fn existing_stroke_width_preserves_attributes() {
        let mut o = closed();
        o.stroke = Some(Stroke {
            paint: Paint::Solid { color: "#123456".into() },
            width: 4,
            opacity: 0.3,
            dash: vec![2, 2],
            cap: crate::object::model::LineCap::Round,
            join: crate::object::model::LineJoin::Bevel,
        });
        let op = edit(&o, "stroke-width", serde_json::json!(2.0), Q).unwrap();
        let ObjectOp::SetStyle { stroke, .. } = op else { panic!("set-style") };
        let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
        assert_eq!(s.width, 16, "2 px * Q(8)");
        assert_eq!(s.paint, Paint::Solid { color: "#123456".into() }, "existing paint kept");
        assert_eq!(s.opacity, 0.3);
        assert_eq!(s.cap, crate::object::model::LineCap::Round);
        assert_eq!(s.dash, vec![2, 2]);
    }

    #[test]
    fn paint_to_stroke_on_borderless_uses_the_core_default() {
        let op = edit(&closed(), "stroke", serde_json::json!({ "kind": "solid", "color": "#abcdef" }), 1.0).unwrap();
        let ObjectOp::SetStyle { stroke, .. } = op else { panic!("set-style") };
        let FieldEdit::Set { value: s } = stroke.unwrap() else { panic!("stroke set") };
        assert_eq!(s.paint, Paint::Solid { color: "#abcdef".into() });
        let core_default = default_inspector_stroke();
        assert_eq!(s.width, core_default.width, "default stroke width is the core's");
        assert_eq!(s.cap, core_default.cap);
        assert_eq!(s.dash, core_default.dash);
    }

    #[test]
    fn sizing_segment_patches_only_one_axis() {
        let op = edit(&closed(), "sizing-w", serde_json::json!({ "kind": "fill" }), Q).unwrap();
        assert_eq!(
            op,
            ObjectOp::SetSizing {
                id: "o1".into(),
                sizing: Some(Sizing { w: AxisSizing::Fill, h: AxisSizing::Hug }),
            }
        );
    }

    #[test]
    fn sizing_fixed_companion_px_requantizes() {
        let op = edit(&closed(), "sizing-w", serde_json::json!(32.0), Q).unwrap();
        let ObjectOp::SetSizing { sizing, .. } = op else { panic!("set-sizing") };
        // 32 px * Q(8) = 256 quantized units.
        assert_eq!(sizing.unwrap().w, AxisSizing::Fixed { value: 256 });
    }

    #[test]
    fn layout_mode_flow_on_free_container_synthesizes_default_flow() {
        let op = edit(&closed(), "layout-mode", Value::String("flow".into()), 1.0).unwrap();
        assert_eq!(op, ObjectOp::SetLayout { id: "o1".into(), layout: Some(default_flow_layout()) });
    }

    #[test]
    fn layout_mode_flow_on_flow_container_is_a_noop() {
        let mut o = closed();
        o.layout = Some(Layout {
            axis: LayoutAxis::Vertical,
            lanes: Lanes::Fill,
            spacing: 128,
            align: Align {
                main: crate::object::model::MainAlign::Center,
                cross: crate::object::model::CrossAlign::Stretch,
            },
        });
        // Re-clicking Flow on a customized container must NOT reset to the default.
        assert_eq!(edit(&o, "layout-mode", Value::String("flow".into()), 1.0), None);
        // Free clears the layout.
        assert_eq!(
            edit(&o, "layout-mode", Value::String("free".into()), 1.0),
            Some(ObjectOp::SetLayout { id: "o1".into(), layout: None })
        );
    }

    #[test]
    fn spacing_patches_only_spacing_requantized() {
        let mut o = closed();
        o.layout = Some(default_flow_layout());
        let op = edit(&o, "spacing", serde_json::json!(24.0), Q).unwrap();
        let ObjectOp::SetLayout { layout, .. } = op else { panic!("set-layout") };
        let layout = layout.unwrap();
        assert_eq!(layout.spacing, 192, "24 px * Q(8)");
        assert_eq!(layout.axis, LayoutAxis::Horizontal, "rest preserved");
    }

    #[test]
    fn layout_field_on_layoutless_object_is_none() {
        assert_eq!(edit(&closed(), "spacing", serde_json::json!(8.0), Q), None);
        assert_eq!(edit(&closed(), "axis", Value::String("vertical".into()), 1.0), None);
    }

    #[test]
    fn font_size_requantizes_and_floors_at_one() {
        let mut o = closed();
        o.text = Some(Text {
            runs: vec![TextRun {
                text: "hi".into(),
                color: Some("#112233".into()),
                size: Some(24),
                bold: false,
                italic: false,
                font: Some("Mono".into()),
            }],
            align: TextAlign::Start,
            valign: crate::object::model::TextVAlign::Top,
        });
        let op = edit(&o, "font-size", serde_json::json!(40.0), Q).unwrap();
        let ObjectOp::SetText { text, .. } = op else { panic!("set-text") };
        // 40 px * Q(8) = 320; the other run fields are preserved.
        let run = &text.unwrap().runs[0];
        assert_eq!(run.size, Some(320));
        assert_eq!(run.font.as_deref(), Some("Mono"));
        assert_eq!(run.color.as_deref(), Some("#112233"));
    }

    #[test]
    fn text_field_on_textless_object_is_none() {
        assert_eq!(edit(&closed(), "font-size", serde_json::json!(8.0), Q), None);
    }

    #[test]
    fn unrecognized_control_is_none() {
        // The transform/resize fields are NOT this helper's; they route elsewhere.
        for id in ["x", "y", "rotation", "rotation-flow", "width", "height", "bogus"] {
            assert_eq!(edit(&closed(), id, serde_json::json!(1.0), 1.0), None, "{id}");
        }
    }
}
