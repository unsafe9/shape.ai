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
    synthesize_create_anchors as synthesize_create_anchors_pure,
};
use crate::object::deform::{
    endpoint_release_ops as endpoint_release_ops_pure, is_open_class_d as is_open_class_d_pure,
};
use crate::object::apply::apply_object_op as apply_object_op_pure;
use crate::object::cascade::{move_ops as move_ops_pure, MoveRoots};
use crate::object::model::Transform3x3;
use crate::object::commands::object_command_catalog_json;
use crate::object::gestures::object_gesture_catalog_json;
use crate::object::drawing::{split_subpath_at as split_subpath_at_pure, Brush};
use crate::object::recognize::{recognize_stroke_object, RecognizeMode};
use crate::object::grouping::{
    double_click_action as double_click_action_pure, has_children as has_children_pure,
    pop_out_op as pop_out_op_pure, ungroup_enabled as ungroup_enabled_pure,
};
use crate::object::merge::merge_open_stroke_ops as merge_open_stroke_ops_pure;
use crate::object::model::{Geometry, Object, ObjectScene};
use crate::object::op::ObjectOp;
use crate::object::primitives::{
    build_primitive as build_primitive_pure,
    build_primitive_from_drag as build_primitive_from_drag_pure,
    build_set_style_op as build_set_style_op_pure, DragSpan, PrimitiveKind,
};
use crate::object::region::{OutlineDeriver, StubOutlineDeriver};
use crate::object::templates::build_template as build_template_pure;
use crate::object::undo::UndoStack;
use crate::fractional::generate_key_between;

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

/// The object-local quantized touch cuts the stroke; returns the whole op batch:
/// `[]` on a miss, `[delete]` when the cut empties the object, else
/// `[edit-geometry, ...follower-reprojection]`.
#[wasm_bindgen]
pub fn partial_erase_ops(scene_json: &str, id: &str, x: i32, y: i32, radius: i32) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(object) = scene.objects.iter().find(|o| o.id == id) else {
        return ok_json(&Vec::<ObjectOp>::new());
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
