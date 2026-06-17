//! The Action→Intent resolver: the ONE place a widget actuation becomes a typed
//! op request the shell authors. It is PURE and unit-tested. The WidgetId prefix
//! is the binding key (`cmd:<id>` / `insp:<control-id>` / `swatch:<hex>`), so this
//! parses the id rather than holding a second catalog.
//!
//! Most inspector controls map their id 1:1 (`insp:name`); the composites that a
//! `Pressed`/`Toggle` action cannot carry a value for encode the value INTO the id
//! and this module decodes it: `insp:align:main=center,cross=end` (a grid cell),
//! `insp:lanes::fill` / `insp:lanes::count` (the lanes parts).

use serde_json::Value;
use shape_scene_core::object::catalog::inspector::{
    InspectorControlValue, InspectorView, InspectorWidget,
};
use shape_ui_core::Action;

use crate::chrome::{CANVAS_DELETE_PREFIX, CANVAS_NEW_ID, CANVAS_PREFIX};
use crate::context_menu::SCRIM_ID as CONTEXT_SCRIM_ID;
use crate::inspector::INSPECTOR_PREFIX;
use crate::settings::SCRIM_ID as SETTINGS_SCRIM_ID;
use crate::template::TEMPLATE_PREFIX;
use crate::toolbar::{CMD_PREFIX, PEN_WIDTH_PREFIX, SWATCH_PREFIX};
use crate::UiModel;

/// The typed request a fired widget maps to. The shell authors each: a `Command`
/// runs the shell's catalog handler (undo/copy/zoom/insert/…), an `InspectorEdit`
/// lowers through scene-core `inspector_edit_op`, etc. The shell NEVER invents an
/// op — it forwards the resolved intent.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// A command-catalog action by id (`undo`, `insert-rectangle`, `zoom-in`, …).
    Command(String),
    /// A palette swatch picked a pen color (`#rrggbb`).
    SelectColor(String),
    /// A stroke-popup size button picked a pen width (px).
    SelectPenWidth(f64),
    /// A template-library row picked a template by id; the shell inserts it.
    ApplyTemplate(String),
    /// The switcher picked a canvas by id; the shell activates it.
    SelectCanvas(String),
    /// The switcher's New button: the shell creates a canvas.
    NewCanvas,
    /// The switcher's Delete button picked a canvas id; the shell deletes it.
    DeleteCanvas(String),
    /// An inspector control wrote a value. `op_kind`/`field`/`unit_scale` are
    /// carried straight from the control so the shell calls the core's
    /// `inspector_edit_op` exactly as the Svelte panel did — no shell-side op math.
    InspectorEdit {
        control_id: String,
        op_kind: String,
        field: Option<String>,
        unit_scale: f64,
        value: Value,
    },
    /// The inspector's `canonicalize` Action button (a button control, no value).
    InspectorAction { control_id: String },
    /// A floating overlay (settings modal / context menu) asked to close — a scrim
    /// press or its own dismiss control. The shell clears the matching open flag.
    Dismiss,
}

/// Map a fired `Action` to an `Intent`, or `None` when the action targets nothing
/// authored (e.g. a hover, or a focus event). `model` resolves an inspector
/// control's catalog metadata (op_kind/field/unit_scale/widget) by id.
pub fn resolve(action: &Action, model: &UiModel) -> Option<Intent> {
    match action {
        Action::Pressed(id) => resolve_pressed(id, model),
        Action::ToggleChanged { id, on } => resolve_toggle(id, model, *on),
        Action::SegmentChanged { id, index } => resolve_segment(id, model, *index),
        Action::TextChanged { id, text } => resolve_text(id, model, text),
        // Slider/Focus are not authored by the toolbar/inspector built-ins here.
        Action::SliderChanged { .. } | Action::Focus(_) => None,
    }
}

/// A `Pressed` on a prefixed id (`cmd:`/`swatch:`/`pen-width:`/`template:`/
/// `canvas:`/`canvas-delete:`/`insp:`), the bare canvas-new button, or an overlay
/// scrim.
fn resolve_pressed(id: &str, model: &UiModel) -> Option<Intent> {
    // A floating overlay's background scrim: a press dismisses it. The context menu
    // ITEMS carry the `cmd:` prefix below, so only the bare scrim hits this.
    if id == SETTINGS_SCRIM_ID || id == CONTEXT_SCRIM_ID {
        return Some(Intent::Dismiss);
    }
    if id == CANVAS_NEW_ID {
        return Some(Intent::NewCanvas);
    }
    if let Some(command) = id.strip_prefix(CMD_PREFIX) {
        return Some(Intent::Command(command.to_string()));
    }
    if let Some(hex) = id.strip_prefix(SWATCH_PREFIX) {
        return Some(Intent::SelectColor(hex.to_string()));
    }
    if let Some(px) = id.strip_prefix(PEN_WIDTH_PREFIX) {
        // The chip id carries the width literally; a malformed one authors nothing.
        return px.parse::<f64>().ok().map(Intent::SelectPenWidth);
    }
    if let Some(template_id) = id.strip_prefix(TEMPLATE_PREFIX) {
        return Some(Intent::ApplyTemplate(template_id.to_string()));
    }
    // A canvas-delete chip carries the target id; check it before the bare switch
    // prefix so `canvas-delete:<id>` doesn't parse as a `canvas:` switch.
    if let Some(canvas_id) = id.strip_prefix(CANVAS_DELETE_PREFIX) {
        return Some(Intent::DeleteCanvas(canvas_id.to_string()));
    }
    if let Some(canvas_id) = id.strip_prefix(CANVAS_PREFIX) {
        return Some(Intent::SelectCanvas(canvas_id.to_string()));
    }
    let payload = id.strip_prefix(INSPECTOR_PREFIX)?;
    // An align grid cell / spaceBetween|stretch toggle: `align:main=..,cross=..`.
    if let Some((control_id, value)) = decode_align(payload) {
        return inspector_edit(model, control_id, value);
    }
    // The only other pressable inspector control is the canonicalize button.
    Some(Intent::InspectorAction {
        control_id: payload.to_string(),
    })
}

/// A `ToggleChanged` carries a bool. It routes to a boolean inspector edit, except
/// the lanes `::fill` toggle which authors a lanes wire value.
fn resolve_toggle(id: &str, model: &UiModel, on: bool) -> Option<Intent> {
    let payload = id.strip_prefix(INSPECTOR_PREFIX)?;
    if let Some(control_id) = payload.strip_suffix("::fill") {
        // The lanes Fill toggle: on → Fill, off → Count (defaulted to 1; the user
        // edits the companion count after). Mirrors the Svelte lanes onEdit.
        let value = if on {
            serde_json::json!({ "kind": "fill" })
        } else {
            serde_json::json!({ "kind": "count", "value": 1 })
        };
        return inspector_edit(model, control_id, value);
    }
    inspector_edit(model, payload, Value::Bool(on))
}

fn resolve_text(id: &str, model: &UiModel, text: &str) -> Option<Intent> {
    let payload = id.strip_prefix(INSPECTOR_PREFIX)?;
    if let Some(control_id) = payload.strip_suffix("::count") {
        // The lanes count field: a positive integer count (≥1, rounded). Lanes::Count
        // is a u32 on the wire, so clamp into a sane track range before converting.
        let count = lanes_count_from_text(text)?;
        let value = serde_json::json!({ "kind": "count", "value": count });
        return inspector_edit(model, control_id, value);
    }
    let control = find_control(model, payload)?;
    // A Paint control's text field carries a hex; author a solid paint value (the
    // panel's hex edit replaces the paint with `{kind:solid,color}`). A Number
    // control commits a numeric string parsed to a number (non-numeric dropped, so
    // ui-core's TextInput stays one type — validation lives here, not in ui-core).
    // Everything else (Name) commits the raw string.
    let value = match control.widget {
        InspectorWidget::Paint => serde_json::json!({ "kind": "solid", "color": text }),
        InspectorWidget::Number { .. } => match text.parse::<f64>() {
            Ok(n) => json_f64(n),
            Err(_) => return None,
        },
        _ => Value::String(text.to_string()),
    };
    edit_from_control(control, value)
}

/// A `SegmentChanged` maps the chosen cell index to the wire value the op expects
/// via the control's catalog options — the same mapping `segmentValue` did in TS,
/// moved into Rust. Sizing segments emit an `AxisSizing` tag; font-weight a bool;
/// the rest a lower-cased token.
fn resolve_segment(id: &str, model: &UiModel, index: usize) -> Option<Intent> {
    let control_id = id.strip_prefix(INSPECTOR_PREFIX)?;
    let control = find_control(model, control_id)?;
    let InspectorWidget::Segment { options } = &control.widget else {
        return None;
    };
    let lower = options.get(index)?.to_lowercase();
    let value = if control_id == "sizing-w" || control_id == "sizing-h" {
        // Hug/Fill carry no payload; Fixed needs a value, defaulted to 0 (the user
        // edits it via the companion number field after).
        if lower == "fixed" {
            serde_json::json!({ "kind": "fixed", "value": 0 })
        } else {
            serde_json::json!({ "kind": lower })
        }
    } else if control_id == "font-weight" {
        Value::Bool(lower == "bold")
    } else {
        Value::String(lower)
    };
    edit_from_control(control, value)
}

/// `align:main=<m>,cross=<c>` → (`"align"`, `{main,cross}`). `None` for any other
/// payload.
fn decode_align(payload: &str) -> Option<(&str, Value)> {
    let rest = payload.strip_prefix("align:")?;
    let (main_kv, cross_kv) = rest.split_once(',')?;
    let main = main_kv.strip_prefix("main=")?;
    let cross = cross_kv.strip_prefix("cross=")?;
    Some(("align", serde_json::json!({ "main": main, "cross": cross })))
}

/// Build an `InspectorEdit` from a control id (looked up in the view) + value.
fn inspector_edit(model: &UiModel, control_id: &str, value: Value) -> Option<Intent> {
    edit_from_control(find_control(model, control_id)?, value)
}

/// Build an `InspectorEdit` carrying the control's op_kind/field/unit_scale; `None`
/// when the control has no op_kind (a read-only display).
fn edit_from_control(control: &InspectorControlValue, value: Value) -> Option<Intent> {
    let op_kind = control.op_kind.as_ref()?.clone();
    Some(Intent::InspectorEdit {
        control_id: control.id.clone(),
        op_kind,
        field: control.field.clone(),
        unit_scale: control.unit_scale,
        value,
    })
}

/// Find an inspector control's resolved value (catalog metadata + current value)
/// in the model's view by id.
fn find_control<'a>(model: &'a UiModel, control_id: &str) -> Option<&'a InspectorControlValue> {
    model
        .inspector_view?
        .sections
        .iter()
        .flat_map(|s| &s.controls)
        .find(|c| c.id == control_id)
}

fn json_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Parse a lanes-count text field into a clamped track count (≥1). `Lanes::Count`
/// is a `u32`; clamping to a sane track range makes the conversion exact (no
/// width-narrowing on any pointer width).
fn lanes_count_from_text(text: &str) -> Option<u32> {
    let n = text.parse::<f64>().ok()?;
    let clamped = n.round().clamp(1.0, MAX_LANES);
    // `clamped` is an integer in [1, MAX_LANES] ⊂ u32, so the conversion is exact.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to [1, MAX_LANES] which fits u32 exactly"
    )]
    Some(clamped as u32)
}

/// The upper bound on a lanes track count; far below `u32::MAX`, so a clamped count
/// converts to `u32` without truncation.
const MAX_LANES: f64 = 1_000.0;

/// True when `view` is empty (canvas/empty selection), so the builder skips the
/// inspector panel.
pub(crate) fn view_is_empty(view: &InspectorView) -> bool {
    view.sections.is_empty()
}
