//! The inspector catalog: the canonical list of object-property controls plus a
//! pure dynamic view that resolves which controls apply to a selection and reads
//! their current values. Sibling to [`commands`](super::commands); like
//! [`ObjectCommand`](super::commands::ObjectCommand) the catalog types are
//! `Serialize`-only — the TS side is hand-declared in the shell phase, NOT ts-rs.
//!
//! Pure (catalog tier): no time/rng/IO. Rotation/scale are read ONLY through
//! [`decompose_affine`] (the single source of truth per the locked design), so a
//! genuinely freeform shape honestly reads rotation 0.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::object::affine::{compose_affine, decompose_affine, AffineDecomposition};
use crate::object::grouping::has_children;
use crate::object::model::{
    AxisSizing, Geometry, Object, ObjectScene, ObjectSelection, Paint, Stroke, Text, Transform3x3,
    GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::region::{LocalBounds, OutlineDeriver};

/// Where a control lives in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectorSection {
    Header,
    Placement,
    Layout,
    Appearance,
    Text,
    Action,
}

/// The widget a control renders as. `Number`'s bounds/step are advisory hints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InspectorWidget {
    Text,
    Toggle,
    Badge,
    Button,
    Paint,
    Number {
        unit: String,
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
    },
    Segment {
        options: Vec<String>,
    },
    Lanes,
    Align9,
}

/// Which roles a control is offered for. The dynamic view filters the static
/// catalog by matching each control's `applies_to` against the selection's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppliesTo {
    Always,
    FreePlaced,
    FlowChild,
    Container,
    FlowContainer,
    HasText,
}

/// A static catalog entry: one inspector control and how it lowers to an op.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorControl {
    pub id: String,
    pub label: String,
    pub section: InspectorSection,
    pub widget: InspectorWidget,
    pub applies_to: AppliesTo,
    /// The op kind this control writes through when it maps 1:1; `None` for a
    /// read-only display (e.g. the role badge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_kind: Option<String>,
    /// The op/decomposition field this control edits; `None` for buttons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Quantized-units-per-px for a numeric control whose stored value is in
    /// quantized units (spacing/stroke-width/font-size = `GEOMETRY_QUANTUM_PER_PX`);
    /// `1.0` for controls already in px/degrees. `inspector_view` divides the stored
    /// value by this to emit logical px; the shell multiplies a px edit back by it.
    pub unit_scale: f64,
    pub description: String,
}

impl InspectorControl {
    fn new(
        id: &str,
        label: &str,
        section: InspectorSection,
        widget: InspectorWidget,
        applies_to: AppliesTo,
        op_kind: Option<&str>,
        field: Option<&str>,
        description: &str,
    ) -> Self {
        InspectorControl {
            id: id.to_string(),
            label: label.to_string(),
            section,
            widget,
            applies_to,
            op_kind: op_kind.map(str::to_string),
            field: field.map(str::to_string),
            unit_scale: 1.0,
            description: description.to_string(),
        }
    }

    /// Mark a control whose stored value is in quantized units; its view value is
    /// emitted in px (stored / scale) and a px edit is authored back (px * scale).
    fn with_unit_scale(mut self, scale: f64) -> Self {
        self.unit_scale = scale;
        self
    }
}

/// Quantized units per logical px — the divisor for the px-denominated numeric
/// controls (spacing/stroke-width/font-size). Read from the kernel constant so the
/// inspector and geometry stay on the same Q.
fn quantum_unit_scale() -> f64 {
    f64::from(GEOMETRY_QUANTUM_PER_PX)
}

fn number(unit: &str, min: Option<f64>, max: Option<f64>, step: f64) -> InspectorWidget {
    InspectorWidget::Number {
        unit: unit.to_string(),
        min,
        max,
        step,
    }
}

fn segment(options: &[&str]) -> InspectorWidget {
    InspectorWidget::Segment {
        options: options.iter().map(|s| s.to_string()).collect(),
    }
}

/// The full static inspector control catalog (v1 inventory), in panel order.
pub fn object_inspector_catalog() -> Vec<InspectorControl> {
    use AppliesTo::*;
    use InspectorSection::*;
    use InspectorWidget::{Align9, Badge, Button, Lanes, Paint, Text as TextWidget, Toggle};
    vec![
        // Header
        InspectorControl::new(
            "name",
            "Name",
            Header,
            TextWidget,
            Always,
            Some("set-meta"),
            Some("name"),
            "Panel display name for the object.",
        ),
        InspectorControl::new(
            "visible",
            "Visible",
            Header,
            Toggle,
            Always,
            Some("set-meta"),
            Some("hidden"),
            "Show or hide the object (toggles the stored `hidden` flag, presented inverted).",
        ),
        InspectorControl::new(
            "locked",
            "Locked",
            Header,
            Toggle,
            Always,
            Some("set-meta"),
            Some("locked"),
            "Lock the object against selection and editing.",
        ),
        InspectorControl::new(
            "role",
            "Role",
            Header,
            Badge,
            Always,
            None,
            None,
            "The object's placement/container role (read-only).",
        ),
        // Placement — free-placed objects own their transform.
        InspectorControl::new(
            "x",
            "X",
            Placement,
            number("px", None, None, 1.0),
            FreePlaced,
            Some("set-transform"),
            Some("translate.x"),
            "Horizontal position (transform translate x).",
        ),
        InspectorControl::new(
            "y",
            "Y",
            Placement,
            number("px", None, None, 1.0),
            FreePlaced,
            Some("set-transform"),
            Some("translate.y"),
            "Vertical position (transform translate y).",
        ),
        InspectorControl::new(
            "width",
            "Width",
            Placement,
            number("px", Some(0.0), None, 1.0),
            FreePlaced,
            Some("set-transform"),
            Some("scale.x"),
            "Width (transform scale x).",
        ),
        InspectorControl::new(
            "height",
            "Height",
            Placement,
            number("px", Some(0.0), None, 1.0),
            FreePlaced,
            Some("set-transform"),
            Some("scale.y"),
            "Height (transform scale y).",
        ),
        InspectorControl::new(
            "rotation",
            "Rotation",
            Placement,
            number("deg", None, None, 1.0),
            FreePlaced,
            Some("set-transform"),
            Some("rotation"),
            "Rotation, read from the transform affine decomposition.",
        ),
        // Placement — a flow child's position is owned by its parent's layout, so
        // it carries sizing instead of x/y.
        InspectorControl::new(
            "sizing-w",
            "Width sizing",
            Placement,
            segment(&["Hug", "Fill", "Fixed"]),
            FlowChild,
            Some("set-sizing"),
            Some("w"),
            "How the child sizes along the width: hug content, fill the track, or a fixed value.",
        )
        .with_unit_scale(quantum_unit_scale()),
        InspectorControl::new(
            "sizing-h",
            "Height sizing",
            Placement,
            segment(&["Hug", "Fill", "Fixed"]),
            FlowChild,
            Some("set-sizing"),
            Some("h"),
            "How the child sizes along the height: hug content, fill the track, or a fixed value.",
        )
        .with_unit_scale(quantum_unit_scale()),
        InspectorControl::new(
            "rotation-flow",
            "Rotation",
            Placement,
            number("deg", None, None, 1.0),
            FlowChild,
            Some("set-transform"),
            Some("rotation"),
            "Rotation, read from the transform affine decomposition.",
        ),
        // Layout — a container chooses Free vs Flow; a Flow container exposes the
        // auto-layout inputs.
        InspectorControl::new(
            "layout-mode",
            "Layout",
            Layout,
            segment(&["Free", "Flow"]),
            Container,
            Some("set-layout"),
            Some("mode"),
            "Free (children keep their own transform) or Flow (the parent arranges children).",
        ),
        InspectorControl::new(
            "axis",
            "Axis",
            Layout,
            segment(&["Horizontal", "Vertical"]),
            FlowContainer,
            Some("set-layout"),
            Some("axis"),
            "Main axis the children flow along.",
        ),
        InspectorControl::new(
            "lanes",
            "Lanes",
            Layout,
            Lanes,
            FlowContainer,
            Some("set-layout"),
            Some("lanes"),
            "Track count: 1 = list, N = grid, Fill = wrap-as-fit.",
        ),
        InspectorControl::new(
            "spacing",
            "Spacing",
            Layout,
            number("px", Some(0.0), None, 1.0),
            FlowContainer,
            Some("set-layout"),
            Some("spacing"),
            "Inter-child gap AND uniform container edge inset.",
        )
        .with_unit_scale(quantum_unit_scale()),
        InspectorControl::new(
            "align",
            "Align",
            Layout,
            Align9,
            FlowContainer,
            Some("set-layout"),
            Some("align"),
            "Main- and cross-axis alignment of the children.",
        ),
        InspectorControl::new(
            "clip",
            "Clip",
            Layout,
            Toggle,
            FlowContainer,
            Some("set-clip"),
            Some("clip"),
            "Clip children to this container's bounds.",
        ),
        // Appearance
        InspectorControl::new(
            "fill",
            "Fill",
            Appearance,
            Paint,
            Always,
            Some("set-style"),
            Some("fill"),
            "Fill paint.",
        ),
        InspectorControl::new(
            "stroke",
            "Stroke",
            Appearance,
            Paint,
            Always,
            Some("set-style"),
            Some("stroke"),
            "Stroke paint.",
        ),
        InspectorControl::new(
            "stroke-width",
            "Stroke width",
            Appearance,
            number("px", Some(0.0), None, 1.0),
            Always,
            Some("set-style"),
            Some("stroke.width"),
            "Stroke width.",
        )
        .with_unit_scale(quantum_unit_scale()),
        // Text
        InspectorControl::new(
            "font",
            "Font",
            Text,
            segment(&["Sans", "Serif", "Mono"]),
            HasText,
            Some("set-text"),
            Some("font"),
            "Font family.",
        ),
        InspectorControl::new(
            "font-size",
            "Size",
            Text,
            number("px", Some(1.0), None, 1.0),
            HasText,
            Some("set-text"),
            Some("size"),
            "Font size.",
        )
        .with_unit_scale(quantum_unit_scale()),
        InspectorControl::new(
            "font-weight",
            "Weight",
            Text,
            segment(&["Regular", "Bold"]),
            HasText,
            Some("set-text"),
            Some("bold"),
            "Font weight.",
        ),
        InspectorControl::new(
            "text-align",
            "Text align",
            Text,
            segment(&["Start", "Center", "End", "Justify"]),
            HasText,
            Some("set-text"),
            Some("align"),
            "Text alignment.",
        ),
        InspectorControl::new(
            "text-color",
            "Text color",
            Text,
            Paint,
            HasText,
            Some("set-text"),
            Some("color"),
            "Text color.",
        ),
        // Action
        InspectorControl::new(
            "canonicalize",
            "Straighten",
            Action,
            Button,
            Always,
            Some("canonicalize"),
            None,
            "Extract the dominant orientation into the transform and re-axis-align the geometry.",
        ),
    ]
}

/// The static catalog serialized to JSON — the seam the shell consumes.
pub fn object_inspector_catalog_json() -> String {
    serde_json::to_string(catalog_slice()).expect("object inspector catalog serializes")
}

/// The catalog built once and borrowed thereafter — it is immutable constant data,
/// so `inspector_view` iterates this slice instead of re-allocating ~150 Strings on
/// every selection-driven refresh. (Pure: lazy-init of constant data, no time/rng/IO.)
fn catalog_slice() -> &'static [InspectorControl] {
    static CATALOG: OnceLock<Vec<InspectorControl>> = OnceLock::new();
    CATALOG.get_or_init(object_inspector_catalog)
}

/// Where an object sits position-wise: a free-placed object owns its transform; a
/// flow child's position is owned by its parent's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    Free,
    FlowChild,
}

/// The resolved role of a single object — the basis for filtering the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorRole {
    pub placement: Placement,
    pub container: bool,
    pub flow_container: bool,
    pub has_text: bool,
}

impl InspectorRole {
    /// Whether a control's `applies_to` is offered for this role.
    fn matches(&self, applies_to: AppliesTo) -> bool {
        match applies_to {
            AppliesTo::Always => true,
            AppliesTo::FreePlaced => self.placement == Placement::Free,
            AppliesTo::FlowChild => self.placement == Placement::FlowChild,
            AppliesTo::Container => self.container,
            AppliesTo::FlowContainer => self.flow_container,
            AppliesTo::HasText => self.has_text,
        }
    }

    /// The human role label shown in the read-only `role` badge.
    fn label(&self) -> &'static str {
        if self.flow_container {
            "Flow container"
        } else if self.container {
            "Free container"
        } else if self.placement == Placement::FlowChild {
            "Flow child"
        } else {
            "Free placed"
        }
    }
}

/// Resolve a single object's role within `scene`.
fn resolve_role(scene: &ObjectScene, obj: &Object) -> InspectorRole {
    let in_flow_parent = obj
        .parent
        .as_deref()
        .and_then(|p| scene.get(p))
        .is_some_and(|parent| parent.layout.is_some());
    let container = has_children(scene, &obj.id);
    InspectorRole {
        placement: if in_flow_parent {
            Placement::FlowChild
        } else {
            Placement::Free
        },
        container,
        flow_container: container && obj.layout.is_some(),
        has_text: obj.text.is_some(),
    }
}

/// A control resolved for the current selection: catalog metadata + its current
/// value (or `null`) and a `mixed` flag for multi-select divergence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorControlValue {
    pub id: String,
    pub label: String,
    pub widget: InspectorWidget,
    /// `null` when unset/not-applicable, or when `mixed`.
    pub value: serde_json::Value,
    /// True when the selected objects diverge on this control's value.
    pub mixed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The control's quantized-units-per-px (carried from the catalog) so the shell
    /// re-quantizes a px edit (`px * unit_scale`) without owning the Q.
    pub unit_scale: f64,
}

/// One section of the dynamic view: its controls in catalog order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorSectionView {
    pub section: InspectorSection,
    pub controls: Vec<InspectorControlValue>,
}

/// The full dynamic inspector view for a selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorView {
    pub role: InspectorRole,
    pub sections: Vec<InspectorSectionView>,
}

/// The per-object derivation shared across that object's controls: the affine
/// decomposition (x/y/rotation/width/height) and the local-AABB region bounds
/// (width/height). Both are computed ONCE per object in [`control_value`] instead
/// of re-decomposing/re-deriving the same matrix and geometry per control.
struct ObjectReadCtx {
    decomposition: AffineDecomposition,
    /// `None` when the geometry has no derivable region.
    bounds: Option<LocalBounds>,
}

impl ObjectReadCtx {
    fn new(obj: &Object, deriver: &impl OutlineDeriver) -> Self {
        ObjectReadCtx {
            decomposition: decompose_affine(&obj.transform),
            bounds: deriver.derive_region(&obj.geometry, 1).ok().map(|r| r.bounds),
        }
    }
}

/// Read a control's current value for a single object. `None` => serialize as
/// JSON `null` (unset / not-applicable). The `role` is passed so the `role` badge
/// reads its label; `ctx` carries the object's one-shot decomposition + region
/// bounds backing the transform/extent controls.
///
/// Honest units: a control carrying a non-1 `unit_scale` stores quantized units, so
/// its read value is divided to logical px (`stored / unit_scale`). width/height
/// read ABSOLUTE px = local-AABB px extent × |decomposed scale on that axis|.
fn read_value(
    control: &InspectorControl,
    obj: &Object,
    role: &InspectorRole,
    ctx: &ObjectReadCtx,
) -> serde_json::Value {
    use serde_json::Value;
    match control.id.as_str() {
        "name" => obj
            .name
            .as_ref()
            .map_or(Value::Null, |s| Value::String(s.clone())),
        // `visible` presents the stored `hidden` flag inverted.
        "visible" => Value::Bool(!obj.hidden),
        "locked" => Value::Bool(obj.locked),
        "role" => Value::String(role.label().to_string()),
        // Rotation/scale/translate are read ONLY through the affine decomposition.
        "x" => json_f64(ctx.decomposition.translate.0),
        "y" => json_f64(ctx.decomposition.translate.1),
        "width" => axis_extent_px(ctx, Axis::X),
        "height" => axis_extent_px(ctx, Axis::Y),
        "rotation" | "rotation-flow" => json_f64(ctx.decomposition.rotation_rad.to_degrees()),
        "sizing-w" => {
            axis_sizing_value(obj.sizing.map_or(AxisSizing::Hug, |s| s.w), control.unit_scale)
        }
        "sizing-h" => {
            axis_sizing_value(obj.sizing.map_or(AxisSizing::Hug, |s| s.h), control.unit_scale)
        }
        "layout-mode" => Value::String(
            if obj.layout.is_some() { "flow" } else { "free" }.to_string(),
        ),
        "axis" => obj
            .layout
            .map_or(Value::Null, |l| serde_json::to_value(l.axis).unwrap_or(Value::Null)),
        "lanes" => obj
            .layout
            .map_or(Value::Null, |l| serde_json::to_value(l.lanes).unwrap_or(Value::Null)),
        "spacing" => obj
            .layout
            .map_or(Value::Null, |l| px_value(l.spacing, control.unit_scale)),
        "align" => obj
            .layout
            .map_or(Value::Null, |l| serde_json::to_value(l.align).unwrap_or(Value::Null)),
        "clip" => Value::Bool(obj.clip.unwrap_or(false)),
        "fill" => paint_value(obj.fill.as_ref().map(|f| &f.paint)),
        "stroke" => paint_value(obj.stroke.as_ref().map(|s| &s.paint)),
        "stroke-width" => obj
            .stroke
            .as_ref()
            .map_or(Value::Null, |s: &Stroke| px_value(s.width, control.unit_scale)),
        "font" => text_run_font(obj.text.as_ref()),
        "font-size" => text_run_size(obj.text.as_ref(), control.unit_scale),
        "font-weight" => text_run_bold(obj.text.as_ref()),
        "text-align" => obj
            .text
            .as_ref()
            .map_or(Value::Null, |t: &Text| {
                serde_json::to_value(t.align).unwrap_or(Value::Null)
            }),
        "text-color" => text_run_color(obj.text.as_ref()),
        // `canonicalize` is a button — no value.
        _ => Value::Null,
    }
}

fn json_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

/// A stored quantized scalar emitted as logical px (`units / unit_scale`). A
/// `unit_scale` of 1.0 leaves the value unchanged.
fn px_value(units: i32, unit_scale: f64) -> serde_json::Value {
    json_f64(f64::from(units) / unit_scale)
}

/// The width vs height axis a placement control reads/edits.
#[derive(Clone, Copy)]
pub enum Axis {
    X,
    Y,
}

/// The local-AABB px extent on `axis` of a derived region's `bounds`. The single
/// basis both the width/height READ and [`resize_axis`] WRITE share, so a resize
/// round-trips the displayed px.
fn bounds_extent_px(bounds: LocalBounds, axis: Axis) -> f64 {
    let extent_units = match axis {
        Axis::X => bounds.max_x - bounds.min_x,
        Axis::Y => bounds.max_y - bounds.min_y,
    };
    f64::from(extent_units) / f64::from(GEOMETRY_QUANTUM_PER_PX)
}

/// Absolute px size on one axis = local AABB px extent × |decomposed scale on that
/// axis|. `null` when the geometry has no derivable region. Reads the precomputed
/// `ctx` (no per-control re-derive / re-decompose).
fn axis_extent_px(ctx: &ObjectReadCtx, axis: Axis) -> serde_json::Value {
    let Some(bounds) = ctx.bounds else {
        return serde_json::Value::Null;
    };
    let extent_px = bounds_extent_px(bounds, axis);
    let factor = match axis {
        Axis::X => ctx.decomposition.scale.0,
        Axis::Y => ctx.decomposition.scale.1,
    };
    json_f64(extent_px * factor.abs())
}

/// Return the transform that makes `obj`'s ABSOLUTE px size on `axis` equal
/// `target_px`, by solving for the scale magnitude that the width/height view reads
/// back: `|scale| = target_px / local_extent_px` (sign preserved so a reflection
/// survives). Other transform components are untouched. A degenerate object (no
/// region, or a zero local extent) returns its transform unchanged. The shell
/// authors the returned matrix as a `set-transform` — it does NO geometry math.
pub fn resize_axis(
    transform: &Transform3x3,
    geometry: &Geometry,
    deriver: &impl OutlineDeriver,
    axis: Axis,
    target_px: f64,
) -> Transform3x3 {
    let Some(region) = deriver.derive_region(geometry, 1).ok() else {
        return *transform;
    };
    let extent_px = bounds_extent_px(region.bounds, axis);
    if extent_px == 0.0 {
        return *transform;
    }
    let d = decompose_affine(transform);
    let target_factor = target_px / extent_px;
    let (sx, sy) = d.scale;
    let scale = match axis {
        Axis::X => (target_factor.copysign(sx), sy),
        Axis::Y => (sx, target_factor.copysign(sy)),
    };
    compose_affine(d.translate, d.rotation_rad, scale, d.skew_rad)
}

/// A sizing segment's value. Hug/Fill serialize verbatim; a Fixed value is emitted
/// in logical px (`stored / unit_scale`) so it reads in the same unit as the other
/// px-denominated controls and the shell's px edit re-quantizes symmetrically.
fn axis_sizing_value(s: AxisSizing, unit_scale: f64) -> serde_json::Value {
    match s {
        AxisSizing::Fixed { value } => {
            serde_json::json!({ "kind": "fixed", "value": f64::from(value) / unit_scale })
        }
        other => serde_json::to_value(other).unwrap_or(serde_json::Value::Null),
    }
}

fn paint_value(paint: Option<&Paint>) -> serde_json::Value {
    paint.map_or(serde_json::Value::Null, |p| {
        serde_json::to_value(p).unwrap_or(serde_json::Value::Null)
    })
}

fn first_run(text: Option<&Text>) -> Option<&crate::object::model::TextRun> {
    text.and_then(|t| t.runs.first())
}

fn text_run_font(text: Option<&Text>) -> serde_json::Value {
    first_run(text).map_or(serde_json::Value::Null, |r| {
        r.font
            .as_ref()
            .map_or(serde_json::Value::Null, |f| serde_json::Value::String(f.clone()))
    })
}

fn text_run_size(text: Option<&Text>, unit_scale: f64) -> serde_json::Value {
    first_run(text).map_or(serde_json::Value::Null, |r| {
        r.size
            .map_or(serde_json::Value::Null, |s| px_value(s, unit_scale))
    })
}

fn text_run_bold(text: Option<&Text>) -> serde_json::Value {
    first_run(text).map_or(serde_json::Value::Null, |r| serde_json::Value::Bool(r.bold))
}

fn text_run_color(text: Option<&Text>) -> serde_json::Value {
    first_run(text).map_or(serde_json::Value::Null, |r| {
        r.color
            .as_ref()
            .map_or(serde_json::Value::Null, |c| serde_json::Value::String(c.clone()))
    })
}

/// Build a [`InspectorControlValue`] for one control over the selected objects:
/// the common value, or `null` + `mixed: true` when they diverge. The per-object
/// decomposition + region bounds are precomputed in `contexts` (parallel to
/// `objects`), so the divergence scan re-derives nothing.
fn control_value(
    control: &InspectorControl,
    objects: &[(&Object, InspectorRole)],
    contexts: &[ObjectReadCtx],
) -> InspectorControlValue {
    let mut entries = objects.iter().zip(contexts);
    let ((first_obj, first_role), first_ctx) = entries.next().expect("non-empty selection");
    let first = read_value(control, first_obj, first_role, first_ctx);
    let mut mixed = false;
    for ((obj, role), ctx) in entries {
        if read_value(control, obj, role, ctx) != first {
            mixed = true;
            break;
        }
    }
    InspectorControlValue {
        id: control.id.clone(),
        label: control.label.clone(),
        widget: control.widget.clone(),
        value: if mixed { serde_json::Value::Null } else { first },
        mixed,
        op_kind: control.op_kind.clone(),
        field: control.field.clone(),
        unit_scale: control.unit_scale,
    }
}

/// The pure dynamic inspector view for a selection (catalog tier; reads
/// rotation/scale only via [`decompose_affine`]). `deriver` is injected (not
/// constructed) — same seam as [`solve_layout`](crate::object::solve_layout) — so
/// width/height read the absolute px extent of the local AABB.
///
/// `scene.ensure_parsed()` should be called before this so geometry-derived role
/// resolution is stable; the view itself does not parse.
pub fn inspector_view(
    scene: &ObjectScene,
    selection: &ObjectSelection,
    deriver: &impl OutlineDeriver,
) -> InspectorView {
    // The selected objects + each one's resolved role. An id that doesn't resolve
    // is dropped (a stale selection entry contributes no controls).
    let selected: Vec<(&Object, InspectorRole)> = match selection {
        ObjectSelection::Canvas => Vec::new(),
        ObjectSelection::Object { id } => scene
            .get(id)
            .map(|o| vec![(o, resolve_role(scene, o))])
            .unwrap_or_default(),
        ObjectSelection::Multi { ids } => ids
            .iter()
            .filter_map(|id| scene.get(id).map(|o| (o, resolve_role(scene, o))))
            .collect(),
    };

    // The view-level role: the single role, or an empty role for canvas/empty/
    // multi (the per-control intersection below already encodes multi-applicability).
    let role = match selected.as_slice() {
        [(_, r)] => *r,
        _ => InspectorRole {
            placement: Placement::Free,
            container: false,
            flow_container: false,
            has_text: false,
        },
    };

    if selected.is_empty() {
        return InspectorView {
            role,
            sections: Vec::new(),
        };
    }

    // The decomposition + region bounds per selected object, derived ONCE here and
    // reused across every control (no per-control re-decompose / re-derive_region).
    let contexts: Vec<ObjectReadCtx> = selected
        .iter()
        .map(|(obj, _)| ObjectReadCtx::new(obj, deriver))
        .collect();

    // A control is shown iff it applies to EVERY selected object's role. Iterate the
    // borrowed static catalog — no per-call catalog allocation.
    let mut sections: Vec<InspectorSectionView> = Vec::new();
    for control in catalog_slice() {
        if !selected.iter().all(|(_, r)| r.matches(control.applies_to)) {
            continue;
        }
        let value = control_value(control, &selected, &contexts);
        match sections.iter_mut().find(|s| s.section == control.section) {
            Some(s) => s.controls.push(value),
            None => sections.push(InspectorSectionView {
                section: control.section,
                controls: vec![value],
            }),
        }
    }

    InspectorView { role, sections }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{
        Align, AxisSizing, CrossAlign, Fill, FillRule, Layout, LayoutAxis, Lanes, MainAlign, Object,
        ObjectSelection, Paint, PathNode, Sizing, SubPath, Text, TextAlign, TextRun, TextVAlign,
    };
    use crate::object::region::StubOutlineDeriver;
    use std::collections::HashSet;

    /// The reference deriver the wasm export injects — tests drive the same seam.
    fn view(scene: &ObjectScene, selection: &ObjectSelection) -> InspectorView {
        inspector_view(scene, selection, &StubOutlineDeriver)
    }

    fn rect(id: &str) -> Object {
        Object::new(
            id,
            "a0",
            Geometry::from_subpaths(
                vec![SubPath {
                    closed: true,
                    nodes: vec![
                        PathNode::corner(0, 0),
                        PathNode::corner(80, 0),
                        PathNode::corner(80, 40),
                        PathNode::corner(0, 40),
                    ],
                }],
                FillRule::EvenOdd,
            ),
        )
    }

    fn scene_with(objects: Vec<Object>) -> ObjectScene {
        ObjectScene {
            objects,
            ..Default::default()
        }
    }

    /// All control ids present in the view, flattened across sections.
    fn control_ids(view: &InspectorView) -> HashSet<String> {
        view.sections
            .iter()
            .flat_map(|s| s.controls.iter().map(|c| c.id.clone()))
            .collect()
    }

    fn find<'a>(view: &'a InspectorView, id: &str) -> Option<&'a InspectorControlValue> {
        view.sections
            .iter()
            .flat_map(|s| &s.controls)
            .find(|c| c.id == id)
    }

    #[test]
    fn catalog_ids_unique_and_op_kinds_real() {
        let catalog = object_inspector_catalog();
        let mut seen = HashSet::new();
        for c in &catalog {
            assert!(seen.insert(c.id.clone()), "duplicate control id {}", c.id);
        }
        // Every op_kind must be a real ObjectOp::kind() discriminant.
        let real: HashSet<&str> = [
            "set-meta",
            "set-transform",
            "set-sizing",
            "set-layout",
            "set-clip",
            "set-style",
            "set-text",
            "canonicalize",
        ]
        .into_iter()
        .collect();
        for c in &catalog {
            if let Some(k) = &c.op_kind {
                assert!(real.contains(k.as_str()), "control {} bad op_kind {k}", c.id);
            }
        }
    }

    /// (D) The canonicalize action's display label is single-language English, like
    /// every other catalog label. It read "Straighten / 방향 정규화" (mixed EN+KR, a
    /// placeholder-looking string); pin it to plain "Straighten" with no Korean and no
    /// "/" separator so the regression can't return.
    #[test]
    fn canonicalize_label_is_english_only() {
        let catalog = object_inspector_catalog();
        let canonicalize = catalog
            .iter()
            .find(|c| c.id == "canonicalize")
            .expect("the canonicalize action control");
        assert_eq!(canonicalize.label, "Straighten");
        assert!(
            !canonicalize.label.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c)),
            "the label must contain no Korean (Hangul syllables)"
        );
        assert!(!canonicalize.label.contains('/'), "the label is a single language, not a slash-joined pair");
    }

    /// The dynamic view round-trips through JSON (Serialize → Deserialize → equal).
    /// The UI layer (`crates/ui`) re-derives its widget tree from a view handed back
    /// across the wasm boundary, so a serde-shape regression that broke the round-trip
    /// would silently drop controls there — it fails HERE instead.
    #[test]
    fn dynamic_view_json_round_trips() {
        let scene = scene_with(vec![{
            let mut p = rect("p");
            p.layout = Some(Layout {
                axis: LayoutAxis::Vertical,
                lanes: Lanes::Fill,
                spacing: 8,
                align: Align { main: MainAlign::SpaceBetween, cross: CrossAlign::Stretch },
            });
            p
        }]);
        let view = view(&scene, &ObjectSelection::Object { id: "p".into() });
        let json = serde_json::to_string(&view).unwrap();
        let back: InspectorView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back, "the inspector view must survive a JSON round-trip");
    }

    #[test]
    fn catalog_json_round_trips() {
        let json = object_inspector_catalog_json();
        assert!(json.starts_with('['));
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            arr.as_array().unwrap().len(),
            object_inspector_catalog().len()
        );
    }

    #[test]
    fn free_object_yields_xy_rotation_no_sizing() {
        let scene = scene_with(vec![rect("r")]);
        let view = view(&scene, &ObjectSelection::Object { id: "r".into() });
        let ids = control_ids(&view);
        // Free placement => x/y/width/height/rotation present.
        for id in ["x", "y", "width", "height", "rotation"] {
            assert!(ids.contains(id), "free object missing {id}");
        }
        // Flow-child sizing segments must NOT show for a free object.
        assert!(!ids.contains("sizing-w"));
        assert!(!ids.contains("sizing-h"));
        assert!(!ids.contains("rotation-flow"));
        // Role badge reads "Free placed".
        assert_eq!(
            find(&view, "role").unwrap().value,
            serde_json::Value::String("Free placed".into())
        );
        // x reads the decomposition translate (identity transform => 0).
        assert_eq!(find(&view, "x").unwrap().value, json_f64(0.0));
    }

    #[test]
    fn flow_child_yields_sizing_no_xy() {
        // A child whose parent has a Layout is a flow child.
        let mut parent = rect("p");
        parent.layout = Some(Layout {
            axis: LayoutAxis::Horizontal,
            lanes: Lanes::Count { value: 1 },
            spacing: 0,
            align: Align {
                main: MainAlign::Start,
                cross: CrossAlign::Start,
            },
        });
        let mut child = rect("c");
        child.parent = Some("p".into());
        let scene = scene_with(vec![parent, child]);

        let view = view(&scene, &ObjectSelection::Object { id: "c".into() });
        let ids = control_ids(&view);
        // Flow child => sizing segments + flow rotation, NO x/y/width/height.
        assert!(ids.contains("sizing-w"));
        assert!(ids.contains("sizing-h"));
        assert!(ids.contains("rotation-flow"));
        for id in ["x", "y", "width", "height", "rotation"] {
            assert!(!ids.contains(id), "flow child should not show {id}");
        }
        assert_eq!(
            find(&view, "role").unwrap().value,
            serde_json::Value::String("Flow child".into())
        );
        // Default sizing (None) reads Hug.
        let sizing_w = &find(&view, "sizing-w").unwrap().value;
        assert_eq!(sizing_w["kind"], serde_json::Value::String("hug".into()));
    }

    #[test]
    fn flow_container_yields_layout_controls() {
        let mut parent = rect("p");
        parent.layout = Some(Layout {
            axis: LayoutAxis::Vertical,
            lanes: Lanes::Count { value: 1 },
            spacing: 16,
            align: Align {
                main: MainAlign::Center,
                cross: CrossAlign::Stretch,
            },
        });
        let mut child = rect("c");
        child.parent = Some("p".into());
        let scene = scene_with(vec![parent, child]);

        let view = view(&scene, &ObjectSelection::Object { id: "p".into() });
        let ids = control_ids(&view);
        for id in ["layout-mode", "axis", "lanes", "spacing", "align", "clip"] {
            assert!(ids.contains(id), "flow container missing {id}");
        }
        assert_eq!(
            find(&view, "role").unwrap().value,
            serde_json::Value::String("Flow container".into())
        );
        // axis reads from the layout.
        assert_eq!(
            find(&view, "axis").unwrap().value,
            serde_json::Value::String("vertical".into())
        );
        // spacing reads honest px: 16 quantized units / Q(8) = 2.0 px.
        assert_eq!(find(&view, "spacing").unwrap().value, json_f64(2.0));
        // layout-mode reads "flow".
        assert_eq!(
            find(&view, "layout-mode").unwrap().value,
            serde_json::Value::String("flow".into())
        );
    }

    #[test]
    fn free_container_shows_layout_mode_not_flow_inputs() {
        // A container with children but no Layout is a Free container: it offers
        // the layout-mode toggle (Container) but NOT the FlowContainer inputs.
        let parent = rect("p");
        let mut child = rect("c");
        child.parent = Some("p".into());
        let scene = scene_with(vec![parent, child]);

        let view = view(&scene, &ObjectSelection::Object { id: "p".into() });
        let ids = control_ids(&view);
        assert!(ids.contains("layout-mode"));
        assert!(!ids.contains("axis"));
        assert!(!ids.contains("spacing"));
        assert_eq!(
            find(&view, "role").unwrap().value,
            serde_json::Value::String("Free container".into())
        );
        assert_eq!(
            find(&view, "layout-mode").unwrap().value,
            serde_json::Value::String("free".into())
        );
    }

    #[test]
    fn text_object_yields_text_section() {
        let mut obj = rect("t");
        obj.text = Some(Text {
            runs: vec![TextRun {
                text: "hi".into(),
                color: Some("#112233".into()),
                size: Some(24),
                bold: true,
                italic: false,
                font: Some("Mono".into()),
            }],
            align: TextAlign::Center,
            valign: TextVAlign::Top,
        });
        let scene = scene_with(vec![obj]);

        let view = view(&scene, &ObjectSelection::Object { id: "t".into() });
        let ids = control_ids(&view);
        for id in ["font", "font-size", "font-weight", "text-align", "text-color"] {
            assert!(ids.contains(id), "text object missing {id}");
        }
        assert_eq!(
            find(&view, "font").unwrap().value,
            serde_json::Value::String("Mono".into())
        );
        // font-size reads honest px: 24 quantized units / Q(8) = 3.0 px.
        assert_eq!(find(&view, "font-size").unwrap().value, json_f64(3.0));
        assert_eq!(find(&view, "font-weight").unwrap().value, serde_json::Value::Bool(true));
        assert_eq!(
            find(&view, "text-align").unwrap().value,
            serde_json::Value::String("center".into())
        );
        assert_eq!(
            find(&view, "text-color").unwrap().value,
            serde_json::Value::String("#112233".into())
        );
    }

    #[test]
    fn non_text_object_hides_text_section() {
        let scene = scene_with(vec![rect("r")]);
        let view = view(&scene, &ObjectSelection::Object { id: "r".into() });
        assert!(!control_ids(&view).contains("font"));
    }

    #[test]
    fn multi_select_marks_differing_values_mixed() {
        // Two free objects with different fills: the shared `fill` control is
        // mixed; the shared `visible` (both true) is not.
        let mut a = rect("a");
        a.fill = Some(Fill {
            paint: Paint::Solid { color: "#ff0000".into() },
            opacity: 1.0,
        });
        let mut b = rect("b");
        b.fill = Some(Fill {
            paint: Paint::Solid { color: "#00ff00".into() },
            opacity: 1.0,
        });
        let scene = scene_with(vec![a, b]);

        let view = view(
            &scene,
            &ObjectSelection::Multi {
                ids: vec!["a".into(), "b".into()],
            },
        );
        let fill = find(&view, "fill").expect("fill present for both");
        assert!(fill.mixed, "diverging fills must be mixed");
        assert_eq!(fill.value, serde_json::Value::Null);

        let visible = find(&view, "visible").expect("visible present for both");
        assert!(!visible.mixed, "matching visibility must not be mixed");
        assert_eq!(visible.value, serde_json::Value::Bool(true));
    }

    #[test]
    fn multi_select_intersects_applicable_controls() {
        // A free object + a flow child: x (FreePlaced) and sizing-w (FlowChild)
        // each apply to only one, so neither survives the intersection; `fill`
        // (Always) does.
        let mut parent = rect("p");
        parent.layout = Some(Layout {
            axis: LayoutAxis::Horizontal,
            lanes: Lanes::Count { value: 1 },
            spacing: 0,
            align: Align {
                main: MainAlign::Start,
                cross: CrossAlign::Start,
            },
        });
        let mut child = rect("c");
        child.parent = Some("p".into());
        let free = rect("f");
        let scene = scene_with(vec![parent, child, free]);

        let view = view(
            &scene,
            &ObjectSelection::Multi {
                ids: vec!["f".into(), "c".into()],
            },
        );
        let ids = control_ids(&view);
        assert!(!ids.contains("x"), "x applies only to the free object");
        assert!(!ids.contains("sizing-w"), "sizing-w applies only to the flow child");
        assert!(ids.contains("fill"), "Always control survives the intersection");
        assert!(ids.contains("canonicalize"));
    }

    #[test]
    fn canvas_selection_is_empty() {
        let scene = scene_with(vec![rect("r")]);
        let view = view(&scene, &ObjectSelection::Canvas);
        assert!(view.sections.is_empty());
    }

    #[test]
    fn rotation_reads_through_decomposition() {
        // A rotated transform reads ~30deg through decompose_affine, not 0.
        let mut obj = rect("r");
        obj.transform = crate::object::affine::compose_affine(
            (0.0, 0.0),
            30.0_f64.to_radians(),
            (1.0, 1.0),
            0.0,
        );
        let scene = scene_with(vec![obj]);
        let view = view(&scene, &ObjectSelection::Object { id: "r".into() });
        let rot = find(&view, "rotation").unwrap().value.as_f64().unwrap();
        assert!((rot - 30.0).abs() < 1e-6, "rotation read {rot}");
    }

    #[test]
    fn sizing_fixed_value_round_trips_in_view() {
        let mut parent = rect("p");
        parent.layout = Some(Layout {
            axis: LayoutAxis::Horizontal,
            lanes: Lanes::Count { value: 1 },
            spacing: 0,
            align: Align {
                main: MainAlign::Start,
                cross: CrossAlign::Start,
            },
        });
        let mut child = rect("c");
        child.parent = Some("p".into());
        child.sizing = Some(Sizing {
            w: AxisSizing::Fixed { value: 120 },
            h: AxisSizing::Fill,
        });
        let scene = scene_with(vec![parent, child]);
        let view = view(&scene, &ObjectSelection::Object { id: "c".into() });
        let w_control = find(&view, "sizing-w").unwrap();
        // The sizing controls carry the Q-scale so a px edit re-quantizes symmetrically.
        assert_eq!(w_control.unit_scale, f64::from(GEOMETRY_QUANTUM_PER_PX));
        let w = &w_control.value;
        assert_eq!(w["kind"], serde_json::Value::String("fixed".into()));
        // A Fixed value reads in logical px: 120 quantized units / Q(8) = 15.0 px.
        assert_eq!(w["value"].as_f64().unwrap(), 15.0);
        let h = &find(&view, "sizing-h").unwrap().value;
        assert_eq!(h["kind"], serde_json::Value::String("fill".into()));
    }

    #[test]
    fn quantized_controls_read_logical_px() {
        // The catalog tags exactly the quantized controls with the Q-scale (the three
        // px-denominated numbers plus the two Fixed-sizing segments); every other
        // control stays 1.0.
        let catalog = object_inspector_catalog();
        let scaled: HashSet<&str> = catalog
            .iter()
            .filter(|c| (c.unit_scale - 1.0).abs() > f64::EPSILON)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            scaled,
            ["spacing", "stroke-width", "font-size", "sizing-w", "sizing-h"]
                .into_iter()
                .collect()
        );
        for c in &catalog {
            if scaled.contains(c.id.as_str()) {
                assert_eq!(c.unit_scale, f64::from(GEOMETRY_QUANTUM_PER_PX));
            } else {
                assert_eq!(c.unit_scale, 1.0, "{} must stay unit_scale 1.0", c.id);
            }
        }

        // A spacing of one px-equivalent (Q quantized units) reads exactly 1.0 px.
        let mut parent = rect("p");
        parent.layout = Some(Layout {
            axis: LayoutAxis::Horizontal,
            lanes: Lanes::Count { value: 1 },
            spacing: GEOMETRY_QUANTUM_PER_PX,
            align: Align { main: MainAlign::Start, cross: CrossAlign::Start },
        });
        let mut child = rect("c");
        child.parent = Some("p".into());
        let scene = scene_with(vec![parent, child]);
        let v = view(&scene, &ObjectSelection::Object { id: "p".into() });
        assert_eq!(find(&v, "spacing").unwrap().value, json_f64(1.0));

        // A stroke width of one px-equivalent reads 1.0 px.
        let mut r = rect("s");
        r.stroke = Some(Stroke {
            paint: Paint::Solid { color: "#000".into() },
            width: GEOMETRY_QUANTUM_PER_PX,
            opacity: 1.0,
            dash: Vec::new(),
            cap: Default::default(),
            join: Default::default(),
        });
        let scene = scene_with(vec![r]);
        let v = view(&scene, &ObjectSelection::Object { id: "s".into() });
        assert_eq!(find(&v, "stroke-width").unwrap().value, json_f64(1.0));

        // A font size of one px-equivalent reads 1.0 px.
        let mut t = rect("t");
        t.text = Some(Text {
            runs: vec![TextRun {
                text: "x".into(),
                color: None,
                size: Some(GEOMETRY_QUANTUM_PER_PX),
                bold: false,
                italic: false,
                font: None,
            }],
            align: TextAlign::Start,
            valign: TextVAlign::Top,
        });
        let scene = scene_with(vec![t]);
        let v = view(&scene, &ObjectSelection::Object { id: "t".into() });
        assert_eq!(find(&v, "font-size").unwrap().value, json_f64(1.0));
    }

    #[test]
    fn width_height_read_absolute_px_at_identity_and_after_scale() {
        // rect() spans (0,0)-(80,40) quantized units => 10 px x 5 px local extent.
        let scene = scene_with(vec![rect("r")]);
        let v = view(&scene, &ObjectSelection::Object { id: "r".into() });
        assert_eq!(find(&v, "width").unwrap().value, json_f64(10.0));
        assert_eq!(find(&v, "height").unwrap().value, json_f64(5.0));

        // A 2x / 3x scale multiplies the absolute extent, NOT the scale multiplier.
        let mut scaled = rect("r");
        scaled.transform = compose_affine((0.0, 0.0), 0.0, (2.0, 3.0), 0.0);
        let scene = scene_with(vec![scaled]);
        let v = view(&scene, &ObjectSelection::Object { id: "r".into() });
        assert_eq!(find(&v, "width").unwrap().value, json_f64(20.0));
        assert_eq!(find(&v, "height").unwrap().value, json_f64(15.0));
    }

    #[test]
    fn resize_axis_sets_the_intended_absolute_dimension() {
        // rect() is 10 px wide / 5 px tall at identity. Resizing width to 25 px and
        // height to 12.5 px must make the inspector read those absolute px back.
        let base = rect("r");
        let t = resize_axis(&base.transform, &base.geometry, &StubOutlineDeriver, Axis::X, 25.0);
        let t = resize_axis(&t, &base.geometry, &StubOutlineDeriver, Axis::Y, 12.5);
        let mut resized = rect("r");
        resized.transform = t;
        let scene = scene_with(vec![resized]);
        let v = view(&scene, &ObjectSelection::Object { id: "r".into() });
        assert_eq!(find(&v, "width").unwrap().value, json_f64(25.0));
        assert_eq!(find(&v, "height").unwrap().value, json_f64(12.5));

        // The resize preserves rotation: resizing a rotated object keeps its angle.
        let mut rotated = rect("r");
        rotated.transform = compose_affine((0.0, 0.0), 30.0_f64.to_radians(), (1.0, 1.0), 0.0);
        let t = resize_axis(&rotated.transform, &rotated.geometry, &StubOutlineDeriver, Axis::X, 40.0);
        let d = decompose_affine(&t);
        assert!((d.rotation_rad - 30.0_f64.to_radians()).abs() < 1e-9);
    }
}
