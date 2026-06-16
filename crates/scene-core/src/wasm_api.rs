//! wasm-bindgen JS bridge, gated behind `cfg(feature = "wasm")`. Every export is a
//! THIN bridge: parse JSON into the pure input type, call the pure function,
//! serialize the result back. No canvas logic lives here.
//!
//! Error policy: these functions never panic across the FFI boundary. A
//! deserialize/serialize failure returns a JSON `{"error": "<message>"}` (a normal
//! `String`, not a thrown exception). Domain failures (unknown id, bad patch) are
//! NOT errors here — they flow through as the `errors` array in the payload.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::wasm_bindgen;

/// Serialize `value`, or fall back to an `{"error": ...}` JSON if serialization
/// fails. Used for every success payload so the boundary stays panic-free.
fn ok_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| error_json(&format!("serialize failed: {e}")))
}

/// Build an `{"error": "<msg>"}` JSON string. Falls back to a hand-built literal
/// to stay infallible.
fn error_json(message: &str) -> String {
    #[derive(Serialize)]
    struct ErrorPayload<'a> {
        error: &'a str,
    }
    serde_json::to_string(&ErrorPayload { error: message })
        .unwrap_or_else(|_| "{\"error\":\"unserializable error\"}".to_string())
}

/// Parse `json` into `T`, mapping a serde error into an `Err(error_json)` so the
/// caller can early-return it as the function's `String` result.
fn parse<T: serde::de::DeserializeOwned>(label: &str, json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| error_json(&format!("invalid {label} JSON: {e}")))
}

// The web client runs the SAME object op-apply / region / templates as the server.
use crate::object::anchor_follow::{
    geometry_follow_ops as geometry_follow_ops_pure,
    resolve_create_release as resolve_create_release_pure,
    synthesize_create_anchors as synthesize_create_anchors_pure,
    synthesize_create_anchors_both as synthesize_create_anchors_both_pure, CreateRelease,
    CreateSnap,
};
use crate::object::deform::{
    endpoint_release_ops as endpoint_release_ops_pure, is_open_class_d as is_open_class_d_pure,
};
use crate::object::apply::apply_object_op as apply_object_op_pure;
use crate::object::cascade::{move_ops as move_ops_pure, MoveRoots};
use crate::object::edit::{
    detach_move_ops as detach_move_ops_pure, duplicate_ops as duplicate_ops_pure,
    move_ops_for_pick as move_ops_for_pick_pure,
};
use crate::object::model::{ObjectSelection, Transform3x3};
use crate::object::selection::{select_all as select_all_pure, valid_selection as valid_selection_pure};
use crate::object::affine::{quantize_units, set_transform_field};
use crate::object::commands::object_command_catalog_json;
use crate::object::gestures::object_gesture_catalog_json;
use crate::object::inspector::{
    inspector_view, object_inspector_catalog_json, resize_axis as resize_axis_pure, Axis,
};
use crate::object::inspector_edit::inspector_edit_op as inspector_edit_op_pure;
use crate::object::drawing::{split_subpath_at as split_subpath_at_pure, Brush};
use crate::object::recognize::{
    recognize_stroke_object, RecognizeMode, CREATE_ANCHOR_REUSE_TOLERANCE_PX,
    MERGE_ENDPOINT_TOLERANCE_PX, MIN_DRAG_EXTENT_PX,
};
use crate::object::grouping::{
    double_click_action as double_click_action_pure, group_ops as group_ops_pure,
    has_children as has_children_pure,
    object_selection_in_scope as object_selection_in_scope_pure, pop_out_op as pop_out_op_pure,
    ungroup_enabled as ungroup_enabled_pure, ungroup_ops as ungroup_ops_pure,
};
use crate::object::merge::merge_open_stroke_ops as merge_open_stroke_ops_pure;
use crate::object::model::{Geometry, Object, ObjectScene};
use crate::object::op::ObjectOp;
use crate::object::primitives::{
    build_primitive as build_primitive_pure,
    build_primitive_from_drag as build_primitive_from_drag_pure,
    build_set_style_op as build_set_style_op_pure, DragSpan, PrimitiveKind,
};
use crate::object::region::{
    object_world_aabb as object_world_aabb_pure, world_to_local_quantized, OutlineDeriver,
    StubOutlineDeriver,
};
use crate::object::templates::{
    build_template as build_template_pure, template_anchor as template_anchor_pure,
};
use crate::object::undo::UndoStack;
use crate::fractional::{
    back_order_key as back_order_key_pure, generate_key_between, key_between as key_between_pure,
    next_order_key as next_order_key_pure, reorder_step_ops as reorder_step_ops_pure,
    ReorderDirection,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectApplyResult<'a> {
    scene: &'a ObjectScene,
    /// The inverse op (for the client undo stack), or null on failure.
    inverse: &'a Option<ObjectOp>,
    errors: &'a [String],
}

/// Runs the canonical object op-apply. On a domain failure the scene is returned
/// unchanged with the message in `errors` and `inverse: null`.
#[wasm_bindgen]
pub fn apply_object_op(scene_json: &str, op_json: &str) -> String {
    let mut scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = scene.ensure_parsed() {
        return error_json(&format!("scene geometry parse failed: {e}"));
    }
    let op: ObjectOp = match parse("op", op_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match apply_object_op_pure(&mut scene, op) {
        Ok(inverse) => ok_json(&ObjectApplyResult {
            scene: &scene,
            inverse: &Some(inverse),
            errors: &[],
        }),
        Err(err) => ok_json(&ObjectApplyResult {
            scene: &scene,
            inverse: &None,
            errors: &[err.to_string()],
        }),
    }
}

/// Reference (stub) outline derivation — fill area / hit-test / selection bound.
#[wasm_bindgen]
pub fn derive_region(geometry_json: &str, flatness: i32) -> String {
    let mut geometry: Geometry = match parse("geometry", geometry_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = geometry.ensure_parsed() {
        return error_json(&format!("geometry parse failed: {e}"));
    }
    match StubOutlineDeriver.derive_region(&geometry, flatness) {
        Ok(region) => ok_json(&region),
        Err(e) => error_json(&format!("degenerate geometry: {e:?}")),
    }
}

#[wasm_bindgen]
pub fn object_command_catalog() -> String {
    object_command_catalog_json()
}

#[wasm_bindgen]
pub fn object_gesture_catalog() -> String {
    object_gesture_catalog_json()
}

#[wasm_bindgen]
pub fn object_inspector_catalog() -> String {
    object_inspector_catalog_json()
}

/// `scene_json` is an `ObjectScene`; `selection_json` is an `ObjectSelection`.
/// Returns the dynamic inspector view (applicable controls + current values).
#[wasm_bindgen]
pub fn object_inspector_view(scene_json: &str, selection_json: &str) -> String {
    let mut scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = scene.ensure_parsed() {
        return error_json(&format!("scene geometry parse failed: {e}"));
    }
    let selection: ObjectSelection = match parse("selection", selection_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&inspector_view(&scene, &selection, &StubOutlineDeriver))
}

/// `matrix_json` is the bare `[[f64; 3]; 3]`; `field` is the inspector control id
/// (`x`/`y` set the translate axis in px, `rotation`/`rotation-flow` set the
/// rotation from the DISPLAY unit — degrees). Returns the patched `Transform3x3`,
/// so the shell never owns the decompose/recompose seam nor the deg->rad math.
/// width/height resize through `object_resize_axis` instead.
#[wasm_bindgen]
pub fn object_set_transform_field(matrix_json: &str, field: &str, value: f64) -> String {
    let t: Transform3x3 = match parse("matrix", matrix_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&set_transform_field(&t, field, value))
}

/// Re-quantize an inspector px edit to the stored i32 via the cores' single
/// `round(px * scale)` discipline. `scale` is the control's `unit_scale`, so the
/// shell owns neither the rounding mode nor the geometry quantum.
#[wasm_bindgen]
pub fn object_quantize_units(px: f64, scale: f64) -> i32 {
    quantize_units(px, scale)
}

/// `transform_json` is a bare `[[f64; 3]; 3]`, `geometry_json` an object `Geometry`,
/// `axis` is `"x"` (width) or `"y"` (height), `target_px` the desired ABSOLUTE px
/// size. Returns the bare `Transform3x3` that makes that axis read `target_px` in
/// the inspector — the geometry/scale math the shell never does. Authored as a
/// `set-transform` op. A bad `axis` token is an error.
#[wasm_bindgen]
pub fn object_resize_axis(
    transform_json: &str,
    geometry_json: &str,
    axis: &str,
    target_px: f64,
) -> String {
    let transform: Transform3x3 = match parse("transform", transform_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut geometry: Geometry = match parse("geometry", geometry_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = geometry.ensure_parsed() {
        return error_json(&format!("geometry parse failed: {e}"));
    }
    let axis = match axis {
        "x" => Axis::X,
        "y" => Axis::Y,
        other => return error_json(&format!("unknown resize axis {other:?}")),
    };
    let t = resize_axis_pure(&transform, &geometry, &StubOutlineDeriver, axis, target_px);
    ok_json(&t)
}

/// Lower ONE inspector-panel property edit to its `ObjectOp`. `object_json` is the
/// edited `Object` (read from the render-only mirror), `control_id` the catalog
/// control id, `value_json` the panel's edit value as the view handed it down, and
/// `unit_scale` the control's `unit_scale` (so a px edit re-quantizes in-core). The
/// per-field op synthesis + every style/layout/sizing/text default a borderless or
/// layout-less object gains live here, not the shell. Returns the op JSON, or `null`
/// for a control this surface does not own (the transform/resize fields x/y/rotation/
/// width/height route through `object_set_transform_field` / `object_resize_axis`) or
/// one whose edit cannot apply (a layout field on a layout-less object). A `null`
/// `value_json` is treated as JSON null.
#[wasm_bindgen]
pub fn object_inspector_edit_op(
    object_json: &str,
    control_id: &str,
    value_json: &str,
    unit_scale: f64,
) -> String {
    let object: Object = match parse("object", object_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let value: serde_json::Value = match parse("value", value_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&inspector_edit_op_pure(&object, control_id, &value, unit_scale))
}

/// Where a new template should land: `[x, y]` world px, `gap_px` right of the
/// right-most object's transform origin and top-aligned, or `[fallback_x,
/// fallback_y]` when the scene is empty. The shell passes its viewport center as
/// the fallback.
#[wasm_bindgen]
pub fn template_anchor(scene_json: &str, fallback_x: f64, fallback_y: f64) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let (x, y) = template_anchor_pure(&scene, fallback_x, fallback_y);
    ok_json(&[x, y])
}

/// A template recipe of inline-styled objects; the shell sends these as a
/// `FeatureRequest::TemplateApply` recipe.
#[wasm_bindgen]
pub fn build_object_template(
    template_id: &str,
    anchor_x: f64,
    anchor_y: f64,
    id_prefix: &str,
) -> String {
    let mut n: u32 = 0;
    let prefix = id_prefix.to_string();
    let mut id_alloc = move || {
        let id = format!("{prefix}-{n}");
        n += 1;
        id
    };
    // Ascending fractional order keys chained from the previous key.
    let mut prev: Option<String> = None;
    let mut order_alloc = move || {
        let key = generate_key_between(prev.as_deref(), None)
            // Fallback appends `~` (sorts after alphanumerics) to stay ascending.
            .unwrap_or_else(|_| match &prev {
                Some(p) => format!("{p}~"),
                None => "a0".to_string(),
            });
        prev = Some(key.clone());
        key
    };
    let objects = build_template_pure(template_id, anchor_x, anchor_y, &mut id_alloc, &mut order_alloc);
    ok_json(&objects)
}

/// Commit a single freehand stroke: `points_json` is a JSON array of `[x, y]`
/// world-px samples, recognized per `mode` (`"basic"` force-snaps to a basic
/// primitive; `"free"` runs the full pipeline) into one object. `{error}` for
/// fewer than 2 points, an unknown mode, or malformed input.
#[wasm_bindgen]
pub fn freehand_to_object(
    points_json: &str,
    color: &str,
    width_px: f64,
    id: &str,
    order: &str,
    mode: &str,
) -> String {
    let mode = match mode {
        "basic" => RecognizeMode::Basic,
        "free" => RecognizeMode::Free,
        _ => return error_json("mode must be \"basic\" or \"free\""),
    };
    let points: Vec<[f64; 2]> = match parse("points", points_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if points.len() < 2 {
        return error_json("freehand needs at least 2 points");
    }
    let pts: Vec<(f64, f64)> = points.iter().map(|p| (p[0], p[1])).collect();
    let object = recognize_stroke_object(
        &pts,
        mode,
        &Brush::new(color, width_px),
        id.to_string(),
        order.to_string(),
    );
    ok_json(&object)
}

/// Multi-stroke endpoint merge: ops merging a released stroke (`points_json`,
/// world-px samples) into the open-class object(s) its ends landed within
/// `tolerance_px` (WORLD px). `null` = no merge (no endpoint hit, or the stroke
/// recognizes closed by itself).
#[wasm_bindgen]
pub fn merge_open_stroke_ops(
    scene_json: &str,
    points_json: &str,
    mode: &str,
    tolerance_px: f64,
) -> String {
    let mode = match mode {
        "basic" => RecognizeMode::Basic,
        "free" => RecognizeMode::Free,
        _ => return error_json("mode must be \"basic\" or \"free\""),
    };
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let points: Vec<[f64; 2]> = match parse("points", points_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let pts: Vec<(f64, f64)> = points.iter().map(|p| (p[0], p[1])).collect();
    ok_json(&merge_open_stroke_ops_pure(&scene, &pts, mode, tolerance_px))
}

/// Partial erase: cut a stroke's geometry at a touched point. `x`/`y`/`radius` are
/// object-local quantized coords. The nearest node within `radius` is removed,
/// splitting its subpath into two open subpaths (degenerate <2-node flanks drop).
/// `{error}` when the touch missed every node or the input was malformed. A simple
/// split, NOT a geometric boolean.
#[wasm_bindgen]
pub fn split_subpath_at(geometry_json: &str, x: i32, y: i32, radius: i32) -> String {
    let mut geometry: Geometry = match parse("geometry", geometry_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = geometry.ensure_parsed() {
        return error_json(&format!("geometry parse failed: {e}"));
    }
    match split_subpath_at_pure(&geometry, x, y, radius) {
        Some(cut) => ok_json(&cut),
        None => error_json("erase touch hit no stroke node"),
    }
}

/// A WORLD touch point (`wx`/`wy`, logical px) cuts the stroke; the core maps it
/// into the object's local quantized space (inverse-affine + quantize, owned by
/// `world_to_local_quantized` — no inverse-affine math in the shell), then cuts.
/// Returns the whole op batch: `[]` on a miss, `[delete]` when the cut empties the
/// object or the transform is singular (can't map the touch), else
/// `[edit-geometry, ...follower-reprojection]`.
/// `radius` is the object-local quantized erase tolerance.
#[wasm_bindgen]
pub fn partial_erase_ops(scene_json: &str, id: &str, wx: f64, wy: f64, radius: i32) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(object) = scene.objects.iter().find(|o| o.id == id) else {
        return ok_json(&Vec::<ObjectOp>::new());
    };
    let Some((x, y)) = world_to_local_quantized(object, wx, wy) else {
        // Singular transform: can't map the touch to cut, so fall back to deleting
        // the whole object (parity with the shell's old det=0 path).
        return ok_json(&vec![ObjectOp::Delete { id: id.into() }]);
    };
    let mut geometry = object.geometry.clone();
    if let Err(e) = geometry.ensure_parsed() {
        return error_json(&format!("geometry parse failed: {e}"));
    }
    let Some(cut) = split_subpath_at_pure(&geometry, x, y, radius) else {
        return ok_json(&Vec::<ObjectOp>::new()); // touch missed
    };
    if cut.path_string.trim().is_empty() {
        return ok_json(&vec![ObjectOp::Delete { id: id.into() }]);
    }
    let edit = ObjectOp::EditGeometry { id: id.into(), geometry: cut };
    let mut ops = vec![edit.clone()];
    ops.extend(geometry_follow_ops_pure(&StubOutlineDeriver, &scene, std::slice::from_ref(&edit)));
    ok_json(&ops)
}

/// The world-space AABB (logical px) of an object: its geometry nodes carried
/// through the transform, min/max'd. This is the geometry half of the shell's old
/// `unionWorldAabb`/text-overlay rect — `{minX,minY,maxX,maxY}`, or `{error}` when
/// the geometry has no nodes or the input is malformed.
#[wasm_bindgen]
pub fn object_world_aabb(object_json: &str) -> String {
    let object: Object = match parse("object", object_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match object_world_aabb_pure(&object) {
        Some(aabb) => ok_json(&aabb),
        None => error_json("object has no geometry nodes"),
    }
}

/// A basic primitive centered on a world anchor, in the toolbar `color` (empty =
/// kind default; may be the theme-default sentinel or a hex). `{error}` for an
/// unknown `kind`.
#[wasm_bindgen]
pub fn build_primitive(kind: &str, anchor_x: f64, anchor_y: f64, color: &str, id: &str, order: &str) -> String {
    let Some(kind) = PrimitiveKind::from_str(kind) else {
        return error_json(&format!("unknown primitive kind: {kind}"));
    };
    let color = (!color.is_empty()).then_some(color);
    let object = build_primitive_pure(kind, anchor_x, anchor_y, color, id, order);
    ok_json(&object)
}

/// A primitive sized to a drag span — closed kinds to the normalized bbox, the
/// line corner-to-corner. Same color rules as [`build_primitive`]. `{error}` for
/// an unknown `kind`.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn build_primitive_from_drag(
    kind: &str,
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    color: &str,
    id: &str,
    order: &str,
) -> String {
    let Some(kind) = PrimitiveKind::from_str(kind) else {
        return error_json(&format!("unknown primitive kind: {kind}"));
    };
    let color = (!color.is_empty()).then_some(color);
    let span = DragSpan { start_x, start_y, end_x, end_y };
    let object = build_primitive_from_drag_pure(kind, span, color, id, order);
    ok_json(&object)
}

/// A `set-style` op recoloring `object` to `color` (hex or the theme-default
/// sentinel). Touches only existing style fields; a borderless object gains a fill
/// so the recolor is visible.
#[wasm_bindgen]
pub fn build_set_style_op(object_json: &str, color: &str) -> String {
    let object: Object = match parse("object", object_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&build_set_style_op_pure(&object, color))
}

/// Commit-time anchor follow. `ops_json` is the committed batch: `set-transform`
/// MOVES a target, `edit-geometry` RESHAPES one. The result is the chord-deform
/// `edit-geometry` ops that make every anchored follower track its target, chained.
/// The standalone entry; `move_ops`/`endpoint_release_ops`/`partial_erase_ops` fold
/// the same follow into their batches. `[]` when nothing follows.
#[wasm_bindgen]
pub fn anchor_follow_ops(scene_json: &str, ops_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ops: Vec<ObjectOp> = match parse("ops", ops_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&geometry_follow_ops_pure(&StubOutlineDeriver, &scene, &ops))
}

/// Wire shape for the [`move_ops`] roots: `{kind:"single",id}` for a single
/// dragged object, or `{kind:"multi",ids}` for a multi-select drag. Decoded here
/// and converted into the pure [`MoveRoots`] (which carries no serde).
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum MoveRootsWire {
    Single { id: String },
    Multi { ids: Vec<String> },
}

impl From<MoveRootsWire> for MoveRoots {
    fn from(wire: MoveRootsWire) -> Self {
        match wire {
            MoveRootsWire::Single { id } => MoveRoots::Single(id),
            MoveRootsWire::Multi { ids } => MoveRoots::Multi(ids),
        }
    }
}

/// The parent-drag / multi-select transform CASCADE ops FOLLOWED BY the
/// anchor-follow `edit-geometry` ops those moves trigger (cascade before follow is
/// a contract). `roots_json` is the [`MoveRootsWire`] shape; `delta_json` is the
/// world-space gesture matrix.
#[wasm_bindgen]
pub fn move_ops(scene_json: &str, roots_json: &str, delta_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let roots: MoveRootsWire = match parse("roots", roots_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let delta: Transform3x3 = match parse("delta", delta_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&move_ops_pure(&scene, &roots.into(), &delta))
}

/// The body-drag commit ops, deriving the [`MoveRoots`] from the live
/// `selection` + picked `id` in-core (a Multi-on-member drag moves the whole
/// set; otherwise the picked single root cascades its subtree). `selection_json`
/// is the `ObjectSelection` wire shape (`{kind:"multi",ids}` / `{kind:"object",id}`
/// / `{kind:"canvas"}`); `delta_json` is the world-space gesture matrix. Cascade
/// before follow, same as [`move_ops`].
#[wasm_bindgen]
pub fn move_ops_for_pick(scene_json: &str, selection_json: &str, id: &str, delta_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let selection: ObjectSelection = match parse("selection", selection_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let delta: Transform3x3 = match parse("delta", delta_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&move_ops_for_pick_pure(&scene, &selection, id, &delta))
}

/// Reconcile `selection_json` against the scene: drop ids no longer present and
/// collapse the kind (`>=2 live -> multi`, `1 -> object`, `0 -> canvas`). Returns
/// the canonical `ObjectSelection` wire shape. The single source of truth for the
/// collapse rule the shell mirrored in TS.
#[wasm_bindgen]
pub fn valid_selection(scene_json: &str, selection_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let selection: ObjectSelection = match parse("selection", selection_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&valid_selection_pure(&scene, &selection))
}

/// Select every object in the scene, collapsed by the same rule as
/// [`valid_selection`]: empty -> `canvas`, one -> `object`, otherwise `multi`.
#[wasm_bindgen]
pub fn select_all(scene_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&select_all_pure(&scene))
}

/// The `insert-object` ops cloning each id in `ids_json` (a JSON string array)
/// with a fresh id (`{id_prefix}-{n}`, indexed from `order_seed`) and a fresh
/// fractional order key, offset by the canonical duplicate translate. Unknown
/// ids are skipped; `[]` when nothing resolves.
#[wasm_bindgen]
pub fn duplicate_ops(scene_json: &str, ids_json: &str, id_prefix: &str, order_seed: u32) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ids: Vec<String> = match parse("ids", ids_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&duplicate_ops_pure(&scene, &ids, id_prefix, order_seed))
}

/// Alt-detach commit ops for an Alt-held body drag of an anchored open-class
/// object: a `set-anchor` clearing its anchors, THEN the single-root
/// [`move_ops`] computed against the scene with that object's anchors already
/// cleared (so the move keeps the 0-rebake whole-object translate, not an
/// anchor-follow reprojection). `delta_json` is the world-space gesture matrix.
/// `[]` when `id` is not in the scene.
#[wasm_bindgen]
pub fn detach_move_ops(scene_json: &str, id: &str, delta_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let delta: Transform3x3 = match parse("delta", delta_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&detach_move_ops_pure(&scene, id, &delta))
}

/// Drag-create anchoring: binds `created`'s node nearest the snapped world
/// endpoint to `target`; `at` is the snap world point in the target's LOCAL
/// quantized space. `null` when no anchor should be authored.
#[wasm_bindgen]
pub fn synthesize_create_anchors(
    created_json: &str,
    target_json: &str,
    endpoint_x: f64,
    endpoint_y: f64,
) -> String {
    let created: Object = match parse("created", created_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let target: Object = match parse("target", target_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let anchors = synthesize_create_anchors_pure(&created, &target, endpoint_x, endpoint_y);
    ok_json(&anchors)
}

/// A create gesture corner on the wire: a snapped `target` id plus the snap world
/// point `{x,y}`, or `null` for an unsnapped corner.
#[derive(Deserialize)]
struct CornerWire {
    target: String,
    x: f64,
    y: f64,
}

/// Release-time anchor authoring for BOTH gesture corners (shape drag-create AND
/// the freehand pen): each corner binds `created`'s nearest node to its snapped
/// target's outline, deduped to ONE anchor per node (first wins). `corners_json` is
/// a 2-element JSON array of `{target,x,y}` | `null`. A null corner or a stale
/// target authors nothing; a degenerate tap whose corners collapse onto one node
/// binds at most one anchor. Returns the (possibly empty) anchor array.
#[wasm_bindgen]
pub fn synthesize_create_anchors_both(
    scene_json: &str,
    created_json: &str,
    corners_json: &str,
) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let created: Object = match parse("created", created_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let corners: Vec<Option<CornerWire>> = match parse("corners", corners_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let corners: Vec<Option<(String, f64, f64)>> = corners
        .into_iter()
        .map(|c| c.map(|c| (c.target, c.x, c.y)))
        .collect();
    let anchors = synthesize_create_anchors_both_pure(&scene, &created, &corners);
    ok_json(&anchors)
}

/// A create-gesture release on the wire: the release endpoint `{x,y}` (world px),
/// whether it landed on a live snap, and that snap's target id (empty = none).
#[derive(Deserialize)]
struct CreateReleaseWire {
    x: f64,
    y: f64,
    snapped: bool,
    target: String,
}

/// The resolved create release on the wire: the endpoint the created node lands on
/// and the anchor target (`null` = author no anchor).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedCreateReleaseWire {
    end: [f64; 2],
    target: Option<String>,
}

/// Resolve a shape drag-create RELEASE to its final endpoint + anchor target: honor
/// the release's own snap, else reuse the gesture's last snap when the release lands
/// within `tolerance_world` (WORLD units) by squared distance. `last_snap_json` is
/// `{x,y,target}` | empty (no prior snap). The reuse radius lives in core
/// (`CREATE_ANCHOR_REUSE_TOLERANCE_PX`); the shell passes it / zoom as
/// `tolerance_world`.
#[wasm_bindgen]
pub fn resolve_create_release(
    release_json: &str,
    last_snap_json: &str,
    tolerance_world: f64,
) -> String {
    let release: CreateReleaseWire = match parse("release", release_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let last_snap: Option<CreateSnap> = if last_snap_json.is_empty() {
        None
    } else {
        #[derive(Deserialize)]
        struct SnapWire {
            x: f64,
            y: f64,
            target: String,
        }
        match parse::<SnapWire>("lastSnap", last_snap_json) {
            Ok(s) => Some(CreateSnap { at: (s.x, s.y), target: s.target }),
            Err(e) => return e,
        }
    };
    let release = CreateRelease {
        end: (release.x, release.y),
        snapped: release.snapped,
        target: (!release.target.is_empty()).then_some(release.target),
    };
    let resolved = resolve_create_release_pure(&release, last_snap.as_ref(), tolerance_world);
    ok_json(&ResolvedCreateReleaseWire {
        end: [resolved.end.0, resolved.end.1],
        target: resolved.target,
    })
}

/// True iff `d` parses to exactly one open subpath. The core classifier
/// class-dependent shell branches consult instead of re-parsing geometry in TS.
#[wasm_bindgen]
pub fn is_open_class_d(d: &str) -> String {
    ok_json(&is_open_class_d_pure(d))
}

/// A world-px point on the wire (`{x,y}`), used by [`endpoint_release_ops`]'s
/// optional snap-at payload.
#[derive(Deserialize)]
struct PointWire {
    x: f64,
    y: f64,
}

/// The commit of an endpoint-drag release: ONE chord-deform `edit-geometry`
/// moving the dragged endpoint to the release point, plus a `set-anchor`
/// whole-vector rewrite (rebind on snap, unbind in empty space). `snap_target_id`
/// empty = no snap; `snap_at_json` is the snapped world point `{x,y}` (empty = the
/// release point). `[]` when the op does not apply (unknown id, closed-class,
/// interior node).
#[wasm_bindgen]
pub fn endpoint_release_ops(
    scene_json: &str,
    id: &str,
    node_index: i32,
    new_x_px: f64,
    new_y_px: f64,
    snap_target_id: &str,
    snap_at_json: &str,
) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let snap_at: Option<(f64, f64)> = if snap_at_json.is_empty() {
        None
    } else {
        match parse::<PointWire>("snapAt", snap_at_json) {
            Ok(p) => Some((p.x, p.y)),
            Err(e) => return e,
        }
    };
    let snap = if snap_target_id.is_empty() {
        None
    } else {
        Some((snap_target_id, snap_at.unwrap_or((new_x_px, new_y_px))))
    };
    let ops = endpoint_release_ops_pure(&scene, id, node_index, (new_x_px, new_y_px), snap);
    ok_json(&fold_follower_reprojection(&scene, ops))
}

/// Append the follower-reprojection ops for any `EditGeometry` in the batch, so one
/// `author()` applies the reshape AND moves its anchored followers.
fn fold_follower_reprojection(scene: &ObjectScene, ops: Vec<ObjectOp>) -> Vec<ObjectOp> {
    let edits: Vec<ObjectOp> =
        ops.iter().filter(|op| matches!(op, ObjectOp::EditGeometry { .. })).cloned().collect();
    if edits.is_empty() {
        return ops;
    }
    let mut all = ops;
    all.extend(geometry_follow_ops_pure(&StubOutlineDeriver, scene, &edits));
    all
}

/// The `reparent` op that pops `id` out one level (to its grandparent, or the
/// canvas root), preserving its order key. `null` when `id` is unknown or already
/// at the root.
#[wasm_bindgen]
pub fn pop_out_op(scene_json: &str, id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&pop_out_op_pure(&scene, id))
}

/// Whether `id` is a container (has at least one child) in the object forest.
#[wasm_bindgen]
pub fn has_children(scene_json: &str, id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&has_children_pure(&scene, id))
}

/// Whether ungroup is enabled: true only when `selected_id` is a container. An
/// empty `selected_id` is treated as no selection.
#[wasm_bindgen]
pub fn ungroup_enabled(scene_json: &str, selected_id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let selected = if selected_id.is_empty() { None } else { Some(selected_id) };
    ok_json(&ungroup_enabled_pure(&scene, selected))
}

/// The container-vs-leaf decision for a double-click on `id`:
/// `{"kind":"drill-in-container"}` when it has children, else `{"kind":"edit-leaf"}`.
#[wasm_bindgen]
pub fn double_click_action(scene_json: &str, id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&double_click_action_pure(&scene, id))
}

/// Whether `selection` still belongs to the active drill-in `container` scope:
/// `"true"` when it is the container itself or a direct child, else `"false"`. The
/// shell mirrors this verdict to keep its active-container token in lockstep,
/// retracting it when this is `"false"`.
#[wasm_bindgen]
pub fn object_selection_in_scope(scene_json: &str, selection_json: &str, container: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let selection: ObjectSelection = match parse("selection", selection_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&object_selection_in_scope_pure(&scene, &selection, container))
}

/// The ops grouping `ids` under a new frame `frame_id`: an `insert-object` for a
/// clipped frame sized + placed to the children's union world-AABB, then one
/// `reparent` per child re-homing it into the frame. `null` when fewer than two
/// known members resolve or no member yields a derivable region. `ids_json` is a
/// JSON string array of object ids.
#[wasm_bindgen]
pub fn group_ops(scene_json: &str, ids_json: &str, frame_id: &str) -> String {
    let mut scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(e) = scene.ensure_parsed() {
        return error_json(&format!("scene geometry parse failed: {e}"));
    }
    let ids: Vec<String> = match parse("ids", ids_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&group_ops_pure(&scene, &ids, frame_id))
}

/// The ops dissolving the container `frame_id`: one `reparent` per child re-homing
/// it to the frame's parent (grandparent or canvas root), then a `delete` of the
/// empty frame. `null` when `frame_id` is unknown.
#[wasm_bindgen]
pub fn ungroup_ops(scene_json: &str, frame_id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&ungroup_ops_pure(&scene, frame_id))
}

/// The order key for a NEW object landing on top (strictly above the scene's max
/// order, the canonical first key on an empty scene), minted through fractional
/// indexing so the shell never invents an `order~` key.
#[wasm_bindgen]
pub fn next_order_key(scene_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&next_order_key_pure(&scene))
}

/// The order key for a NEW object landing at the back (strictly below the
/// scene's min order). Replaces the shell's `0`-prefixed key invention.
#[wasm_bindgen]
pub fn back_order_key(scene_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&back_order_key_pure(&scene))
}

/// A key strictly between `a` and `b` (empty = open end). `{error}` when a bound
/// is malformed or `a >= b`.
#[wasm_bindgen]
pub fn key_between(a: &str, b: &str) -> String {
    let lo = (!a.is_empty()).then_some(a);
    let hi = (!b.is_empty()).then_some(b);
    match key_between_pure(lo, hi) {
        Ok(key) => ok_json(&key),
        Err(e) => error_json(&e),
    }
}

/// The 2-op `reorder` swap stepping `id` one place toward the front
/// (`"forward"`) or back (`"backward"`) over the flat scene order. `null` when
/// `id` is unknown or has no neighbor in that direction. `{error}` for an
/// unknown direction.
#[wasm_bindgen]
pub fn reorder_step_ops(scene_json: &str, id: &str, direction: &str) -> String {
    let direction = match direction {
        "forward" => ReorderDirection::Forward,
        "backward" => ReorderDirection::Backward,
        _ => return error_json("direction must be \"forward\" or \"backward\""),
    };
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&reorder_step_ops_pure(&scene, id, direction))
}

/// The create-gesture screen-px thresholds the shell owns no copy of: the
/// click-vs-drag extent, the release anchor-reuse radius, and the stroke
/// merge-endpoint radius. The shell reads them here and divides by zoom — the
/// values live ONLY in `recognize`.
#[wasm_bindgen]
pub fn create_thresholds() -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Thresholds {
        min_drag_extent_px: f64,
        create_anchor_reuse_tolerance_px: f64,
        merge_endpoint_tolerance_px: f64,
    }
    ok_json(&Thresholds {
        min_drag_extent_px: MIN_DRAG_EXTENT_PX,
        create_anchor_reuse_tolerance_px: CREATE_ANCHOR_REUSE_TOLERANCE_PX,
        merge_endpoint_tolerance_px: MERGE_ENDPOINT_TOLERANCE_PX,
    })
}

// Stateful wrapper over the core `UndoStack`: undo/redo hand out an op JSON for
// the host to re-author through the SAME op-apply path, then the host reports the
// re-inverse via `note_*_applied`. The `note_*` methods are guarded by a mirrored
// `pending` flag so an out-of-order call returns `false` instead of hitting the
// core's `panic!`; malformed op JSON returns `false` as well.

/// Which handshake the wrapper is awaiting, mirroring the core's private
/// `Pending` so the `note_*` calls can be guarded against an FFI panic.
#[derive(Clone, Copy, PartialEq)]
enum PendingKind {
    Undo,
    Redo,
}

#[wasm_bindgen]
pub struct WasmUndoStack {
    inner: UndoStack,
    pending: Option<PendingKind>,
}

#[wasm_bindgen]
impl WasmUndoStack {
    /// `actor_id` is informational (the crate stores it).
    #[wasm_bindgen(constructor)]
    pub fn new(actor_id: &str) -> WasmUndoStack {
        WasmUndoStack { inner: UndoStack::new(actor_id.to_string()), pending: None }
    }

    /// Clears redo (a fresh edit forks history); folds into the live entry during
    /// a coalescing window. `false` if either op JSON is malformed.
    pub fn record(&mut self, forward_json: &str, inverse_json: &str) -> bool {
        let forward: ObjectOp = match serde_json::from_str(forward_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let inverse: ObjectOp = match serde_json::from_str(inverse_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        self.inner.record(forward, inverse);
        true
    }

    /// Open a coalescing window so a continuous gesture (e.g. a drag) collapses
    /// to one undo step.
    pub fn begin_coalesce(&mut self) {
        self.inner.begin_coalesce();
    }

    /// Close the coalescing window, committing its single entry (if any).
    pub fn end_coalesce(&mut self) {
        self.inner.end_coalesce();
    }

    pub fn is_coalescing(&self) -> bool {
        self.inner.is_coalescing()
    }

    /// Begin an undo: returns the inverse op JSON to apply through
    /// `apply_object_op`, or `null` when nothing to undo. The caller MUST then
    /// apply it and report the re-inverse via [`note_undo_applied`].
    ///
    /// [`note_undo_applied`]: Self::note_undo_applied
    pub fn undo(&mut self) -> Option<String> {
        let op = self.inner.undo()?;
        self.pending = Some(PendingKind::Undo);
        Some(ok_json(&op))
    }

    /// Complete the undo handshake with the re-inverse `apply_object_op`
    /// returned. Returns `false` if no undo handshake is in flight or the JSON
    /// is malformed (never panics across the boundary).
    pub fn note_undo_applied(&mut self, re_inverse_json: &str) -> bool {
        if self.pending != Some(PendingKind::Undo) {
            return false;
        }
        let re_inverse: ObjectOp = match serde_json::from_str(re_inverse_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        self.inner.note_undo_applied(re_inverse);
        self.pending = None;
        true
    }

    /// Begin a redo: returns the op JSON to re-apply, or `null` when nothing to
    /// redo. The caller MUST apply it and report the inverse via
    /// [`note_redo_applied`].
    ///
    /// [`note_redo_applied`]: Self::note_redo_applied
    pub fn redo(&mut self) -> Option<String> {
        let op = self.inner.redo()?;
        self.pending = Some(PendingKind::Redo);
        Some(ok_json(&op))
    }

    /// Complete the redo handshake with the inverse `apply_object_op` returned.
    /// Returns `false` if no redo handshake is in flight or the JSON is
    /// malformed.
    pub fn note_redo_applied(&mut self, inverse_json: &str) -> bool {
        if self.pending != Some(PendingKind::Redo) {
            return false;
        }
        let inverse: ObjectOp = match serde_json::from_str(inverse_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        self.inner.note_redo_applied(inverse);
        self.pending = None;
        true
    }

    /// Abort an in-flight undo/redo handshake whose apply did not succeed (domain
    /// error or wire failure). Clears the wrapper's mirrored `pending` and the
    /// core's, so the stack stays consistent and future undo/redo keep working.
    pub fn abort(&mut self) {
        self.pending = None;
        self.inner.abort_pending();
    }

    pub fn can_undo(&self) -> bool {
        self.inner.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.inner.can_redo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::Object;

    #[test]
    fn freehand_recognizes_a_straight_stroke_as_a_two_node_open_line() {
        let json = freehand_to_object(
            "[[0,0],[5,0],[10,0]]",
            "#1f2933",
            2.0,
            "draw-1",
            "a0",
            "free",
        );
        let object: Object = serde_json::from_str(&json).expect("freehand returns a valid Object");
        assert_eq!(object.id, "draw-1");
        // A straight stroke commits as the canonical 2-node line.
        assert_eq!(object.geometry.path_string, "M 0 0 L 80 0");
        assert!(object.stroke.is_some(), "freehand commit carries a stroke");
    }

    #[test]
    fn freehand_mode_gates_basic_snap_vs_free_pipeline() {
        // A zigzag the Free pipeline keeps as a multi-node open path but the
        // Basic mode force-snaps to the 2-node chord line.
        let zigzag = "[[0,0],[25,40],[50,0],[75,40],[100,0]]";
        let basic = freehand_to_object(zigzag, "#1f2933", 2.0, "d1", "a0", "basic");
        let object: Object = serde_json::from_str(&basic).expect("basic returns a valid Object");
        assert_eq!(object.geometry.path_string, "M 0 0 L 800 0");
        let free = freehand_to_object(zigzag, "#1f2933", 2.0, "d2", "a0", "free");
        let object: Object = serde_json::from_str(&free).expect("free returns a valid Object");
        assert_ne!(object.geometry.path_string, "M 0 0 L 800 0", "free keeps the zigzag");
        // Unknown modes error instead of silently picking a pipeline.
        let bad = freehand_to_object(zigzag, "#1f2933", 2.0, "d3", "a0", "diagonal");
        assert!(bad.contains("\"error\""), "unknown mode is an error");
    }

    #[test]
    fn freehand_rejects_too_few_points() {
        let json = freehand_to_object("[[0,0]]", "#000000", 1.0, "d", "a0", "basic");
        assert!(json.contains("\"error\""), "single point is an error");
    }

    #[test]
    fn merge_open_stroke_ops_bridges_ops_or_null() {
        // A scene with one open line (0,0)->(100,0)px: a stroke released ON its
        // end merges (an edit-geometry batch, no insert); one far away is null.
        let scene = r#"{"sceneVersion":1,"objects":[{"id":"seg","order":"a0","geometry":{"d":"M 0 0 L 800 0"}}],"tags":[],"selection":{"kind":"canvas"},"updatedAt":""}"#;
        let merged =
            merge_open_stroke_ops(scene, "[[101,1],[150,0],[200,0]]", "basic", 12.0);
        let ops: Vec<ObjectOp> = serde_json::from_str(&merged).expect("merge returns ops");
        assert_eq!(ops.len(), 1, "{merged}");
        assert!(matches!(&ops[0], ObjectOp::EditGeometry { id, .. } if id == "seg"));
        let far = merge_open_stroke_ops(scene, "[[500,500],[600,500]]", "basic", 12.0);
        assert_eq!(far, "null", "no endpoint hit = no merge");
        let bad = merge_open_stroke_ops(scene, "[[0,0],[1,1]]", "diagonal", 12.0);
        assert!(bad.contains("\"error\""), "unknown mode is an error");
    }

    #[test]
    fn synthesize_create_anchors_both_bridge_binds_dedupes_and_skips() {
        // rect-a spans x in [200,300] via a drag-built rect; an open line whose node
        // 0 sits at world (200,30) and node 1 at (500,30).
        let scene = r#"{"sceneVersion":1,"objects":[
            {"id":"rect-a","order":"a0","transform":[[1,0,200],[0,1,0],[0,0,1]],"geometry":{"d":"M 0 0 L 800 0 L 800 480 L 0 480 Z"}},
            {"id":"rect-b","order":"a0","transform":[[1,0,500],[0,1,0],[0,0,1]],"geometry":{"d":"M 0 0 L 800 0 L 800 480 L 0 480 Z"}}
        ],"tags":[],"selection":{"kind":"canvas"},"updatedAt":""}"#;
        let created = r#"{"id":"edge","order":"a1","transform":[[1,0,0],[0,1,0],[0,0,1]],"geometry":{"d":"M 1600 240 L 4000 240"}}"#;
        let out = synthesize_create_anchors_both(
            scene,
            created,
            r#"[{"target":"rect-a","x":200,"y":30},{"target":"rect-b","x":500,"y":30}]"#,
        );
        let anchors: Vec<crate::object::model::Anchor> =
            serde_json::from_str(&out).expect("both returns anchors");
        assert_eq!(anchors.len(), 2, "one anchor per corner: {out}");
        assert_eq!(anchors[0].node_index, 0);
        assert_eq!(anchors[0].target, "rect-a");
        assert_eq!(anchors[1].node_index, 1);
        assert_eq!(anchors[1].target, "rect-b");

        // A null corner + a stale-target corner author nothing.
        let none = synthesize_create_anchors_both(
            scene,
            created,
            r#"[null,{"target":"rect-gone","x":500,"y":30}]"#,
        );
        assert_eq!(
            serde_json::from_str::<Vec<crate::object::model::Anchor>>(&none).unwrap().len(),
            0,
            "null + stale corners bind nothing: {none}"
        );
    }

    #[test]
    fn resolve_create_release_bridge_reuses_within_radius_and_drops_outside() {
        // A live snap on the release wins outright.
        let live = resolve_create_release(
            r#"{"x":200,"y":30,"snapped":true,"target":"rect-b"}"#,
            r#"{"x":10,"y":10,"target":"old"}"#,
            24.0,
        );
        let v: serde_json::Value = serde_json::from_str(&live).unwrap();
        assert_eq!(v["target"], "rect-b");
        assert_eq!(v["end"][0].as_f64().unwrap(), 200.0);

        // A miss INSIDE the reuse radius reuses the snap point + target.
        let reuse = resolve_create_release(
            r#"{"x":318,"y":0,"snapped":false,"target":""}"#,
            r#"{"x":300,"y":0,"target":"rect-a"}"#,
            24.0,
        );
        let v: serde_json::Value = serde_json::from_str(&reuse).unwrap();
        assert_eq!(v["target"], "rect-a", "within radius reuses: {reuse}");
        assert_eq!(v["end"][0].as_f64().unwrap(), 300.0, "snap point, not the release end");

        // A miss OUTSIDE the radius authors no anchor and keeps the release endpoint.
        let drop = resolve_create_release(
            r#"{"x":400,"y":0,"snapped":false,"target":""}"#,
            r#"{"x":300,"y":0,"target":"rect-a"}"#,
            24.0,
        );
        let v: serde_json::Value = serde_json::from_str(&drop).unwrap();
        assert!(v["target"].is_null(), "outside the radius authors no anchor: {drop}");
        assert_eq!(v["end"][0].as_f64().unwrap(), 400.0);

        // No prior snap at all: the release passes through unbound.
        let unbound =
            resolve_create_release(r#"{"x":50,"y":60,"snapped":false,"target":""}"#, "", 24.0);
        let v: serde_json::Value = serde_json::from_str(&unbound).unwrap();
        assert!(v["target"].is_null());
        assert_eq!(v["end"][1].as_f64().unwrap(), 60.0);
    }

    #[test]
    fn partial_erase_ops_returns_the_whole_op_batch() {
        // A touch near the middle node cuts the stroke; a far touch misses; an
        // unknown id is a no-op.
        let scene = r#"{"sceneVersion":1,"objects":[{"id":"seg","order":"a0","geometry":{"d":"M 0 0 L 80 0 L 160 0 L 240 0 L 320 0"}}],"tags":[],"selection":{"kind":"canvas"},"updatedAt":""}"#;
        let cut = partial_erase_ops(scene, "seg", 161, 1, 16);
        let ops: Vec<ObjectOp> = serde_json::from_str(&cut).expect("erase returns ops");
        assert!(
            ops.iter().any(|op| matches!(op, ObjectOp::EditGeometry { id, .. } if id == "seg")),
            "a cut authors an edit-geometry: {cut}"
        );
        let miss = partial_erase_ops(scene, "seg", 5000, 5000, 16);
        assert_eq!(serde_json::from_str::<Vec<ObjectOp>>(&miss).unwrap().len(), 0, "miss = no ops: {miss}");
        let gone = partial_erase_ops(scene, "nope", 0, 0, 16);
        assert_eq!(serde_json::from_str::<Vec<ObjectOp>>(&gone).unwrap().len(), 0, "missing object = no ops");
    }

    #[test]
    fn split_subpath_at_cuts_geometry_into_two_open_subpaths() {
        // A 5-node open polyline; cutting near the middle node yields a geometry
        // whose path-string has two `M` subpaths and no `Z` (both open).
        let geometry = r#"{"d":"M 0 0 L 80 0 L 160 0 L 240 0 L 320 0"}"#;
        let out = split_subpath_at(geometry, 161, 1, 16);
        let cut: Geometry = serde_json::from_str(&out).expect("cut returns a Geometry");
        assert_eq!(cut.path_string.matches('M').count(), 2, "two subpaths");
        assert!(!cut.path_string.contains('Z'), "both pieces are open");
    }

    #[test]
    fn split_subpath_at_errors_when_touch_misses() {
        let geometry = r#"{"d":"M 0 0 L 80 0"}"#;
        let out = split_subpath_at(geometry, 5000, 5000, 16);
        assert!(out.contains("\"error\""), "a missed touch is an error");
    }

    // --- move_ops bridge ---

    /// A two-node line object, optionally parented, in the camelCase wire shape.
    fn line_object_json(id: &str, parent: Option<&str>, tx: f64, ty: f64) -> String {
        let parent_field = parent.map(|p| format!(r#""parent":"{p}","#)).unwrap_or_default();
        format!(
            r#"{{"id":"{id}",{parent_field}"order":"a0","transform":[[1,0,{tx}],[0,1,{ty}],[0,0,1]],"geometry":{{"d":"M 0 0 L 8 0"}}}}"#
        )
    }

    fn scene_json(objects: &[String]) -> String {
        format!(r#"{{"objects":[{}]}}"#, objects.join(","))
    }

    /// The ordered set-transform ids from a `move_ops` result JSON.
    fn move_ops_ids(out: &str) -> Vec<String> {
        let ops: serde_json::Value = serde_json::from_str(out).expect("move_ops returns ops");
        ops.as_array()
            .unwrap()
            .iter()
            .filter(|op| op["kind"] == "set-transform")
            .map(|op| op["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn move_ops_bridge_single_root_cascades_subtree() {
        let scene = scene_json(&[
            line_object_json("frame", None, 100.0, 100.0),
            line_object_json("c1", Some("frame"), 110.0, 120.0),
        ]);
        let roots = r#"{"kind":"single","id":"frame"}"#;
        let delta = "[[1,0,40],[0,1,25],[0,0,1]]";
        let out = move_ops(&scene, roots, delta);
        assert_eq!(move_ops_ids(&out), vec!["frame", "c1"]);
        // The child shifted by exactly the delta: 110+40, 120+25.
        let ops: serde_json::Value = serde_json::from_str(&out).unwrap();
        let c1 = ops.as_array().unwrap().iter().find(|op| op["id"] == "c1").unwrap();
        assert_eq!(c1["transform"][0][2].as_f64().unwrap(), 150.0);
        assert_eq!(c1["transform"][1][2].as_f64().unwrap(), 145.0);
    }

    #[test]
    fn move_ops_bridge_multi_root_dedupes_in_input_order() {
        // a{b}, d{e}; multi-select [a,d] => ["a","b","d","e"] (matches renderer-core).
        let scene = scene_json(&[
            line_object_json("a", None, 0.0, 0.0),
            line_object_json("b", Some("a"), 0.0, 0.0),
            line_object_json("d", None, 0.0, 0.0),
            line_object_json("e", Some("d"), 0.0, 0.0),
        ]);
        let roots = r#"{"kind":"multi","ids":["a","d"]}"#;
        let out = move_ops(&scene, roots, "[[1,0,1],[0,1,1],[0,0,1]]");
        assert_eq!(move_ops_ids(&out), vec!["a", "b", "d", "e"]);
    }

    #[test]
    fn move_ops_bridge_rejects_malformed_roots() {
        let scene = scene_json(&[line_object_json("a", None, 0.0, 0.0)]);
        let out = move_ops(&scene, "not json", "[[1,0,0],[0,1,0],[0,0,1]]");
        assert!(out.contains("\"error\""), "malformed roots is a bridge error");
    }

    // --- move_ops_for_pick / duplicate_ops / detach_move_ops bridges ---

    #[test]
    fn move_ops_for_pick_bridge_multi_member_moves_the_set() {
        // [a,b] multi, picking `a`: both members move (vs single `a` only).
        let scene = scene_json(&[
            line_object_json("a", None, 0.0, 0.0),
            line_object_json("b", None, 50.0, 0.0),
        ]);
        let selection = r#"{"kind":"multi","ids":["a","b"]}"#;
        let out = move_ops_for_pick(&scene, selection, "a", "[[1,0,10],[0,1,0],[0,0,1]]");
        assert_eq!(move_ops_ids(&out), vec!["a", "b"]);
        // Picking a non-member cascades only that single root.
        let single = move_ops_for_pick(&scene, selection, "b", "[[1,0,10],[0,1,0],[0,0,1]]");
        assert_eq!(move_ops_ids(&single), vec!["a", "b"], "b is a member, still moves the set");
        let outside = move_ops_for_pick(
            &scene_json(&[line_object_json("c", None, 0.0, 0.0)]),
            r#"{"kind":"object","id":"c"}"#,
            "c",
            "[[1,0,10],[0,1,0],[0,0,1]]",
        );
        assert_eq!(move_ops_ids(&outside), vec!["c"], "single selection = single root");
    }

    #[test]
    fn duplicate_ops_bridge_authors_inserts_with_fresh_ids_order_and_offset() {
        let scene = scene_json(&[line_object_json("a", None, 100.0, 50.0)]);
        let out = duplicate_ops(&scene, r#"["a"]"#, "dup", 0);
        let ops: serde_json::Value = serde_json::from_str(&out).expect("dup returns ops");
        let arr = ops.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], "insert-object");
        assert_eq!(arr[0]["object"]["id"], "dup-0", "fresh id");
        // +40/+40 from the source's (100,50).
        let t = &arr[0]["object"]["transform"];
        assert_eq!(t[0][2].as_f64().unwrap(), 140.0);
        assert_eq!(t[1][2].as_f64().unwrap(), 90.0);
        // Fresh order sorts strictly above the scene top ("a0").
        assert!(arr[0]["object"]["order"].as_str().unwrap() > "a0");
    }

    #[test]
    fn detach_move_ops_bridge_clears_anchor_then_translates_whole() {
        // An open line anchored to a rect: detach clears the anchor and keeps a
        // whole-object set-transform (no anchor-follow edit-geometry).
        let edge = r#"{"id":"edge","order":"a1","transform":[[1,0,0],[0,1,0],[0,0,1]],"geometry":{"d":"M 0 0 L 800 0"},"anchors":[{"nodeIndex":0,"target":"rect","at":{"x":0,"y":0}}]}"#.to_string();
        let scene = scene_json(&[line_object_json("rect", None, 0.0, 0.0), edge]);
        let out = detach_move_ops(&scene, "edge", "[[1,0,40],[0,1,30],[0,0,1]]");
        let ops: serde_json::Value = serde_json::from_str(&out).expect("detach returns ops");
        let arr = ops.as_array().expect("array");
        assert_eq!(arr[0]["kind"], "set-anchor");
        assert_eq!(arr[0]["id"], "edge");
        assert!(arr[0]["anchors"].as_array().unwrap().is_empty(), "anchors cleared first");
        assert!(
            arr.iter().any(|op| op["kind"] == "set-transform" && op["id"] == "edge"),
            "whole-object translate: {out}"
        );
        assert!(
            !arr.iter().any(|op| op["kind"] == "edit-geometry" && op["id"] == "edge"),
            "no anchor-follow reprojection of the detached edge: {out}"
        );
    }

    // --- grouping bridges ---

    /// A scene with `root -> mid -> deep` plus a root-level `leaf`.
    fn grouping_scene_json() -> String {
        scene_json(&[
            line_object_json("root", None, 0.0, 0.0),
            line_object_json("mid", Some("root"), 0.0, 0.0),
            line_object_json("deep", Some("mid"), 0.0, 0.0),
            line_object_json("leaf", None, 0.0, 0.0),
        ])
    }

    #[test]
    fn pop_out_op_bridge_authors_reparent_to_grandparent() {
        let out = pop_out_op(&grouping_scene_json(), "deep");
        let op: serde_json::Value = serde_json::from_str(&out).expect("op json");
        assert_eq!(op["kind"], "reparent");
        assert_eq!(op["id"], "deep");
        assert_eq!(op["parent"], "root");
        assert_eq!(op["order"], "a0");
    }

    #[test]
    fn pop_out_op_bridge_is_null_for_root_level() {
        // `leaf` sits at the root -> null (nothing to pop out of).
        assert_eq!(pop_out_op(&grouping_scene_json(), "leaf"), "null");
    }

    #[test]
    fn has_children_bridge_reports_container() {
        assert_eq!(has_children(&grouping_scene_json(), "root"), "true");
        assert_eq!(has_children(&grouping_scene_json(), "leaf"), "false");
    }

    #[test]
    fn ungroup_enabled_bridge_gates_on_children_and_selection() {
        assert_eq!(ungroup_enabled(&grouping_scene_json(), "mid"), "true");
        assert_eq!(ungroup_enabled(&grouping_scene_json(), "leaf"), "false");
        // An empty selected-id string is no selection.
        assert_eq!(ungroup_enabled(&grouping_scene_json(), ""), "false");
    }

    #[test]
    fn double_click_action_bridge_branches_container_vs_leaf() {
        let drill: serde_json::Value =
            serde_json::from_str(&double_click_action(&grouping_scene_json(), "root")).unwrap();
        assert_eq!(drill["kind"], "drill-in-container");
        let edit: serde_json::Value =
            serde_json::from_str(&double_click_action(&grouping_scene_json(), "leaf")).unwrap();
        assert_eq!(edit["kind"], "edit-leaf");
    }

    #[test]
    fn object_selection_in_scope_bridge_verdict() {
        // root -> mid -> deep + a root-level leaf. The active scope is "root".
        let scene = grouping_scene_json();
        let object = |id: &str| format!(r#"{{"kind":"object","id":"{id}"}}"#);
        // A direct child of root stays.
        assert_eq!(object_selection_in_scope(&scene, &object("mid"), "root"), "true");
        // A grandchild (deep under mid under root) is NOT a direct child -> exits.
        assert_eq!(object_selection_in_scope(&scene, &object("deep"), "root"), "false");
        // The container itself stays; an unrelated root-level leaf exits.
        assert_eq!(object_selection_in_scope(&scene, &object("root"), "root"), "true");
        assert_eq!(object_selection_in_scope(&scene, &object("leaf"), "root"), "false");
        // A canvas selection exits.
        assert_eq!(
            object_selection_in_scope(&scene, r#"{"kind":"canvas"}"#, "root"),
            "false"
        );
    }

    #[test]
    fn group_ops_bridge_authors_insert_frame_plus_reparents() {
        // Two root-level members with distinct orders.
        let a = r#"{"id":"a","order":"a0","transform":[[1,0,0],[0,1,0],[0,0,1]],"geometry":{"d":"M 0 0 L 8 0"}}"#.to_string();
        let b = r#"{"id":"b","order":"a1","transform":[[1,0,5],[0,1,0],[0,0,1]],"geometry":{"d":"M 0 0 L 8 0"}}"#.to_string();
        let scene = scene_json(&[a, b]);
        let out = group_ops(&scene, r#"["a","b"]"#, "frame");
        let ops: serde_json::Value = serde_json::from_str(&out).expect("ops json");
        let arr = ops.as_array().expect("array");
        assert_eq!(arr.len(), 3, "insert frame + 2 reparents");
        assert_eq!(arr[0]["kind"], "insert-object");
        assert_eq!(arr[0]["object"]["id"], "frame");
        assert_eq!(arr[1]["kind"], "reparent");
        assert_eq!(arr[1]["parent"], "frame");
        assert_eq!(arr[2]["kind"], "reparent");
        assert_eq!(arr[2]["parent"], "frame");
    }

    #[test]
    fn group_ops_bridge_is_null_below_two_members() {
        let scene = scene_json(&[line_object_json("a", None, 0.0, 0.0)]);
        assert_eq!(group_ops(&scene, r#"["a"]"#, "frame"), "null");
    }

    #[test]
    fn ungroup_ops_bridge_authors_reparents_then_delete() {
        let out = ungroup_ops(&grouping_scene_json(), "mid");
        let ops: serde_json::Value = serde_json::from_str(&out).expect("ops json");
        let arr = ops.as_array().expect("array");
        // `mid` has one child (`deep`) -> one reparent to grandparent (`root`) + delete.
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["kind"], "reparent");
        assert_eq!(arr[0]["id"], "deep");
        assert_eq!(arr[0]["parent"], "root");
        assert_eq!(arr[1]["kind"], "delete");
        assert_eq!(arr[1]["id"], "mid");
    }

    #[test]
    fn ungroup_ops_bridge_is_null_for_unknown_frame() {
        assert_eq!(ungroup_ops(&grouping_scene_json(), "ghost"), "null");
    }

    // --- WasmUndoStack bridge ---

    /// A `set-transform` op JSON to `(tx, ty)` in the camelCase wire shape.
    fn set_transform_json(tx: f64, ty: f64) -> String {
        format!(
            r#"{{"kind":"set-transform","id":"r","transform":[[1,0,{tx}],[0,1,{ty}],[0,0,1]]}}"#
        )
    }

    /// A one-rect scene at the identity transform, applied through the wasm bridge.
    fn scene_with_rect_json() -> String {
        let insert = r#"{"kind":"insert-object","object":{"id":"r","order":"a0","geometry":{"d":"M 0 0 L 80 0 L 80 40 L 0 40 Z"}}}"#;
        let result: serde_json::Value =
            serde_json::from_str(&apply_object_op("{\"objects\":[]}", insert)).expect("insert ok");
        result["scene"].to_string()
    }

    /// Apply `op_json` to `scene_json` through the bridge and return
    /// `(next_scene_json, inverse_json)`.
    fn apply(scene_json: &str, op_json: &str) -> (String, String) {
        let result: serde_json::Value =
            serde_json::from_str(&apply_object_op(scene_json, op_json)).expect("apply ok");
        assert!(result["errors"].as_array().unwrap().is_empty(), "no domain errors");
        (result["scene"].to_string(), result["inverse"].to_string())
    }

    fn translate_of(scene_json: &str) -> (f64, f64) {
        let scene: serde_json::Value = serde_json::from_str(scene_json).unwrap();
        let t = &scene["objects"][0]["transform"];
        (t[0][2].as_f64().unwrap(), t[1][2].as_f64().unwrap())
    }

    /// record -> undo -> apply -> note -> redo -> apply -> note round-trips
    /// through the SAME op-apply path, leaving the scene where each step expects.
    #[test]
    fn wrapper_undo_redo_round_trips_through_apply() {
        let mut scene = scene_with_rect_json();
        let forward = set_transform_json(5.0, 5.0);
        let (next, inverse) = apply(&scene, &forward);
        scene = next;
        assert_eq!(translate_of(&scene), (5.0, 5.0));

        let mut stack = WasmUndoStack::new("actor-1");
        assert!(stack.record(&forward, &inverse));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());

        // Undo: the bridge hands out the inverse; applying it returns to identity.
        let undo_op = stack.undo().expect("undo available");
        let (next, re_inverse) = apply(&scene, &undo_op);
        scene = next;
        assert!(stack.note_undo_applied(&re_inverse));
        assert_eq!(translate_of(&scene), (0.0, 0.0));
        assert!(!stack.can_undo());
        assert!(stack.can_redo());

        // Redo: hands back the original forward; applying re-reaches the edit.
        let redo_op = stack.redo().expect("redo available");
        let (next, inv_again) = apply(&scene, &redo_op);
        scene = next;
        assert!(stack.note_redo_applied(&inv_again));
        assert_eq!(translate_of(&scene), (5.0, 5.0));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    /// A coalescing window collapses a stream of records to ONE undo step that
    /// lands at the pre-gesture state and redoes to the gesture's final state.
    #[test]
    fn wrapper_coalesced_gesture_is_single_undo_step() {
        let mut scene = scene_with_rect_json();
        let mut stack = WasmUndoStack::new("dragger");

        stack.begin_coalesce();
        assert!(stack.is_coalescing());
        for step in 1..=3 {
            let tx = f64::from(step) * 10.0;
            let forward = set_transform_json(tx, 0.0);
            let (next, inverse) = apply(&scene, &forward);
            scene = next;
            assert!(stack.record(&forward, &inverse));
        }
        stack.end_coalesce();
        assert!(!stack.is_coalescing());
        assert!(stack.can_undo());

        // One undo lands all the way back at identity (kept the FIRST inverse).
        let undo_op = stack.undo().expect("undo");
        let (next, re_inverse) = apply(&scene, &undo_op);
        scene = next;
        assert!(stack.note_undo_applied(&re_inverse));
        assert_eq!(translate_of(&scene), (0.0, 0.0));
        assert!(!stack.can_undo());

        // One redo replays the gesture's final state (kept the LATEST forward).
        let redo_op = stack.redo().expect("redo");
        let (next, inv) = apply(&scene, &redo_op);
        scene = next;
        assert!(stack.note_redo_applied(&inv));
        assert_eq!(translate_of(&scene), (30.0, 0.0));
    }

    /// A fresh record after an undo forks history and clears redo.
    #[test]
    fn wrapper_record_clears_redo() {
        let mut scene = scene_with_rect_json();
        let f1 = set_transform_json(1.0, 0.0);
        let (next, i1) = apply(&scene, &f1);
        scene = next;
        let mut stack = WasmUndoStack::new("a");
        stack.record(&f1, &i1);

        let undo_op = stack.undo().expect("undo");
        let (next, ri) = apply(&scene, &undo_op);
        scene = next;
        stack.note_undo_applied(&ri);
        assert!(stack.can_redo());

        let f2 = set_transform_json(2.0, 0.0);
        let (next, i2) = apply(&scene, &f2);
        scene = next;
        stack.record(&f2, &i2);
        assert!(!stack.can_redo());
    }

    /// `undo`/`redo` return `null` on an empty stack; `note_*` without a
    /// handshake returns `false` instead of panicking across the FFI boundary.
    #[test]
    fn wrapper_guards_against_ffi_panics() {
        let mut stack = WasmUndoStack::new("a");
        assert!(stack.undo().is_none());
        assert!(stack.redo().is_none());
        // No handshake in flight: must not panic, returns false.
        assert!(!stack.note_undo_applied(&set_transform_json(0.0, 0.0)));
        assert!(!stack.note_redo_applied(&set_transform_json(0.0, 0.0)));
        // Malformed op JSON is rejected, not panicked.
        assert!(!stack.record("not json", "also not"));
    }
}
