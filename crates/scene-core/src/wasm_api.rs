//! wasm-bindgen JS bridge (MG0.3/MG0.4) — gated behind `cfg(feature = "wasm")`.
//!
//! Every export here is a THIN bridge: parse a JSON string into the pure
//! scene-core input type, call the corresponding pure function, and serialize the
//! result back to a JSON String. No canvas logic lives here — the goal is that the
//! web client runs the *same* op-apply as the server, so this file only adapts
//! types across the FFI boundary.
//!
//! Error policy: these functions never panic across the FFI boundary. A
//! deserialize/serialize failure is returned as a JSON object
//! `{"error": "<message>"}` (a normal `String` return, not a thrown JS
//! exception), so the TS loader can branch on the `error` field instead of
//! wrapping every call in try/catch. Domain-level failures (an unknown id, a
//! bad patch) are NOT errors here: they flow through normally as the `errors`
//! array inside the returned payload, exactly as the pure functions report them.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::wasm_bindgen;

/// Serialize `value`, or fall back to an `{"error": ...}` JSON if serialization
/// fails. Used for every success payload so the boundary stays panic-free.
fn ok_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| error_json(&format!("serialize failed: {e}")))
}

/// Build an `{"error": "<msg>"}` JSON string. `serde_json::to_string` of a
/// single string field cannot fail, but fall back to a hand-built literal to
/// keep this infallible regardless.
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

// ---------------------------------------------------------------------------
// Object-model bridges (OB-3/OB-4). The single set of bridges after the OB4.4
// legacy removal: the web client runs the SAME object op-apply / region /
// templates as the server (P1: one core, the shell carries no domain logic).
// ---------------------------------------------------------------------------

use crate::object::anchor_follow::{
    anchor_follow_ops as anchor_follow_ops_pure,
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
use crate::object::drawing::{split_subpath_at as split_subpath_at_pure, Brush, DrawingSession};
use crate::object::grouping::{
    double_click_action as double_click_action_pure, has_children as has_children_pure,
    pop_out_op as pop_out_op_pure, ungroup_enabled as ungroup_enabled_pure,
};
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

/// `apply_object_op(scene_json, op_json) -> {scene, inverse, errors}`.
///
/// Runs the canonical object op-apply (the SAME path the server runs). On a
/// domain failure the scene is returned unchanged with the message in `errors`
/// and `inverse: null`, mirroring the render-patch bridge's error policy.
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

/// `derive_region(geometry_json, flatness) -> Region | {error}`.
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

/// `object_command_catalog() -> ObjectCommand[]` (label/category/shortcut/op).
#[wasm_bindgen]
pub fn object_command_catalog() -> String {
    object_command_catalog_json()
}

/// `object_gesture_catalog() -> ObjectGesture[]` (id/label/category/hold-trigger).
#[wasm_bindgen]
pub fn object_gesture_catalog() -> String {
    object_gesture_catalog_json()
}

/// `build_object_template(template_id, anchor_x, anchor_y, id_prefix) -> Object[]`.
/// Builds a template recipe of inline-styled objects; the shell sends these as a
/// `FeatureRequest::TemplateApply` recipe (server lowers them to insert-object ops).
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
    // Deterministic ascending fractional order keys (no rng): chain from the
    // previous key so siblings stay ordered.
    let mut prev: Option<String> = None;
    let mut order_alloc = move || {
        let key = generate_key_between(prev.as_deref(), None)
            // `generate_key_between(Some, None)` does not fail for valid keys;
            // the fallback appends `~` (sorts after alphanumerics) to stay ascending.
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

/// `freehand_to_object(points_json, color, width_px, epsilon, id, order) -> Object | {error}`.
///
/// Capture a single freehand stroke (FC-11): `points_json` is a JSON array of
/// `[x, y]` world-px samples. The session origin is the min (x, y) over the
/// points, so the committed object's geometry is object-local and the origin
/// rides the transform translate (P4 zero-rebake). Returns `{error}` for fewer
/// than 2 points (a tap has no extent) or a malformed input.
#[wasm_bindgen]
pub fn freehand_to_object(
    points_json: &str,
    color: &str,
    width_px: f64,
    epsilon: f64,
    id: &str,
    order: &str,
) -> String {
    let points: Vec<[f64; 2]> = match parse("points", points_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if points.len() < 2 {
        return error_json("freehand needs at least 2 points");
    }
    let origin_x = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let origin_y = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let mut session = DrawingSession::new(Brush::new(color, width_px));
    session.begin_stroke();
    for [x, y] in &points {
        session.push_point(*x, *y);
    }
    session.end_stroke(epsilon);
    let object = session.commit(id.to_string(), order.to_string(), origin_x, origin_y);
    ok_json(&object)
}

/// `split_subpath_at(geometry_json, x, y, radius) -> Geometry | {error}`.
///
/// Partial erase (W2-08/D4): cut a stroke's geometry at a touched point. `x`/`y`/
/// `radius` are object-local quantized coords (the shell converts the world touch
/// into the object's local space). The node nearest the touch within `radius` is
/// removed, splitting its subpath into two open subpaths; degenerate (<2-node)
/// flanks drop. Returns the new geometry, or `{error}` when the touch missed
/// every node (nothing to cut) or the input was malformed — the shell then leaves
/// the stroke unchanged. This is a SIMPLE split, NOT a geometric boolean.
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

/// `build_primitive(kind, anchor_x, anchor_y, color, id, order) -> Object | {error}`.
///
/// Tier-3: build a basic primitive (rectangle/ellipse/line/text/frame) centered on
/// a world anchor, in the toolbar `color` (an empty string means the kind default).
/// `color` may be the theme-default sentinel (resolves to a text token) or a hex.
/// The geometry is object-local; the world position rides a translate (P4). The
/// shell sends the returned object as an `insert-object` op. `{error}` for an
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

/// `build_primitive_from_drag(kind, start_x, start_y, end_x, end_y, color, id, order) -> Object | {error}`.
///
/// Tier-3: build a primitive sized to a drag span — closed kinds to the normalized
/// bbox, the line corner-to-corner. Same color/anchor rules as [`build_primitive`].
/// `{error}` for an unknown `kind`.
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

/// `build_set_style_op(object_json, color) -> ObjectOp | {error}`.
///
/// Tier-3: author a `set-style` op recoloring `object` to `color` (a hex or the
/// theme-default sentinel). Recolor touches only existing style fields; a borderless
/// object gains a fill so the recolor is visible. The shell authors the op through
/// the same op-apply path (D21 undo via the captured inverse).
#[wasm_bindgen]
pub fn build_set_style_op(object_json: &str, color: &str) -> String {
    let object: Object = match parse("object", object_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&build_set_style_op_pure(&object, color))
}

/// `anchor_follow_ops(scene_json, transform_ops_json) -> ObjectOp[] | {error}`.
///
/// Tier-1/#14 commit-time anchor move-together. `transform_ops_json` is the move's
/// `set-transform` ops; the result is the `edit-geometry` ops that reproject every
/// object anchored to a moved target through that target's NEW transform (the SAME
/// reproject the renderer-core LIVE preview applies). Returns `[]` when nothing
/// follows. The shell batches the returned ops into the committed move.
#[wasm_bindgen]
pub fn anchor_follow_ops(scene_json: &str, transform_ops_json: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let transform_ops: Vec<ObjectOp> = match parse("transformOps", transform_ops_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&anchor_follow_ops_pure(&scene, &transform_ops))
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

/// `move_ops(scene_json, roots_json, delta_json) -> ObjectOp[] | {error}`.
///
/// Tier-2 combined commit entry: the parent-drag / multi-select transform CASCADE
/// ops FOLLOWED BY the anchor-follow `edit-geometry` ops those moves trigger, as
/// ONE batch-ready Vec (cascade BEFORE follow — a contract). `roots_json` is the
/// [`MoveRootsWire`] shape (`{kind:"single",id} | {kind:"multi",ids}`); `delta_json`
/// is the world-space gesture matrix (a row-major 3x3). Collapses the shell commit
/// to a single core call.
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

/// `synthesize_create_anchors(created_json, target_json, endpoint_x, endpoint_y)
/// -> Anchor[] | null | {error}`.
///
/// AP5 drag-create anchoring. Binds `created`'s node nearest the snapped world
/// endpoint to `target`; `at` is the snap world point in the target's LOCAL
/// quantized space. Returns `null` when no anchor should be authored (the target
/// is the created object, or the created geometry has no node), which the shell
/// treats as "no anchor" (the Alt-create / no-snap case).
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

/// `is_open_class_d(d) -> bool`.
///
/// Anchor-semantics v3 §1: the open/closed data-level dichotomy for a path
/// string — true iff it parses to exactly one subpath and that subpath is open.
/// Class-dependent shell branches (Alt-detach, fill-vs-stroke routing) consult
/// THE core classifier instead of re-parsing geometry in TS.
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

/// `endpoint_release_ops(scene_json, id, node_index, new_x_px, new_y_px,
/// snap_target_id, snap_at_json) -> ObjectOp[] | {error}`.
///
/// Anchor-semantics v3 §2b: the commit of an endpoint-drag release — ONE
/// chord-deform `edit-geometry` moving the dragged endpoint to the world-px
/// release point, plus the `set-anchor` whole-vector rewrite (rebind when the
/// release snapped, unbind when it landed in empty space). `snap_target_id`
/// empty = no snap; `snap_at_json` is the snapped world point `{x,y}` (empty =
/// the release point itself). Returns `[]` when the op does not apply (unknown
/// id, closed-class, interior node). The shell batches the returned ops.
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
    ok_json(&endpoint_release_ops_pure(&scene, id, node_index, (new_x_px, new_y_px), snap))
}

// ---------------------------------------------------------------------------
// Grouping bridges (Tier-4) — group-hierarchy containment op-generation + forest
// queries. The shell's context-menu pop-out, the ungroup-enabled menu gate, and
// the double-click container-vs-leaf decision now run THE core query; the shell
// keeps only the dispatch (author the op / set active-container / inline edit).
// ---------------------------------------------------------------------------

/// `pop_out_op(scene_json, id) -> ObjectOp | null | {error}`.
///
/// Author the `reparent` op that pops `id` out one level (to its grandparent, or
/// to the canvas root when the parent sits at the root), preserving its order key.
/// Returns `null` when `id` is unknown or already at the root (nothing to pop out
/// of). The shell authors the returned op through the same op-apply path (whose
/// `reparent` arm carries the cycle check).
#[wasm_bindgen]
pub fn pop_out_op(scene_json: &str, id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&pop_out_op_pure(&scene, id))
}

/// `has_children(scene_json, id) -> bool | {error}`.
/// Whether `id` is a container (has at least one child) in the object forest.
#[wasm_bindgen]
pub fn has_children(scene_json: &str, id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&has_children_pure(&scene, id))
}

/// `ungroup_enabled(scene_json, selected_id) -> bool | {error}`.
///
/// Whether ungroup is enabled for the single selected object: true only when a
/// non-null `selected_id` is a container (has children). An empty `selected_id`
/// string is treated as no selection (the canvas / multi-select case the shell
/// gates out before calling).
#[wasm_bindgen]
pub fn ungroup_enabled(scene_json: &str, selected_id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let selected = if selected_id.is_empty() { None } else { Some(selected_id) };
    ok_json(&ungroup_enabled_pure(&scene, selected))
}

/// `double_click_action(scene_json, id) -> DoubleClickAction | {error}`.
///
/// The container-vs-leaf decision for a double-click on object `id`:
/// `{"kind":"drill-in-container"}` when it has children, else
/// `{"kind":"edit-leaf"}`. The shell drives this off the renderer's double-click
/// signal id and dispatches the action (set active-container vs inline text edit).
#[wasm_bindgen]
pub fn double_click_action(scene_json: &str, id: &str) -> String {
    let scene: ObjectScene = match parse("scene", scene_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    ok_json(&double_click_action_pure(&scene, id))
}

// ---------------------------------------------------------------------------
// Undo/redo bridge (FC-15). The per-actor `UndoStack` (D21) now lives in the
// core; the shell drives it through this stateful wrapper instead of a TS
// reimplementation. The wrapper mirrors the crate's semantics exactly: undo/redo
// hand out an op JSON for the host to re-author through the SAME op-apply path,
// then the host reports the re-inverse back via `note_undo_applied` /
// `note_redo_applied`. All op payloads cross the FFI as ObjectOp JSON.
//
// Like the rest of this file the wrapper never panics across the boundary: the
// `note_*` methods are guarded by an internally-mirrored `pending` flag, so an
// out-of-order call (no handshake in flight) returns `false` instead of hitting
// the core's `panic!`. Malformed op JSON returns `false` as well.
// ---------------------------------------------------------------------------

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
    /// Create a per-actor undo stack. `actor_id` is informational (the crate
    /// stores it); the shell passes its authoring identity.
    #[wasm_bindgen(constructor)]
    pub fn new(actor_id: &str) -> WasmUndoStack {
        WasmUndoStack { inner: UndoStack::new(actor_id.to_string()), pending: None }
    }

    /// Record an applied edit: `forward` is what was applied, `inverse` is what
    /// `apply_object_op` returned for it. Clears redo (a fresh edit forks
    /// history); folds into the live entry during a coalescing window. Returns
    /// `true` on success, `false` if either op JSON is malformed.
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
    fn freehand_round_trips_to_open_path_object_with_stroke() {
        let json = freehand_to_object(
            "[[0,0],[5,0],[10,0]]",
            "#1f2933",
            2.0,
            2.0,
            "draw-1",
            "a0",
        );
        let object: Object = serde_json::from_str(&json).expect("freehand returns a valid Object");
        assert_eq!(object.id, "draw-1");
        let d = &object.geometry.path_string;
        assert!(!d.is_empty(), "geometry path is non-empty");
        assert!(d.starts_with('M') && !d.contains('Z'), "open path");
        assert!(object.stroke.is_some(), "freehand commit carries a stroke");
    }

    #[test]
    fn freehand_rejects_too_few_points() {
        let json = freehand_to_object("[[0,0]]", "#000000", 1.0, 1.0, "d", "a0");
        assert!(json.contains("\"error\""), "single point is an error");
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

    // --- move_ops bridge (Tier-2) ---

    /// A two-node line object at translate `(tx, ty)`, optionally parented, in the
    /// camelCase wire shape the shell sends.
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

    // --- grouping bridges (Tier-4) ---

    /// A scene with `root -> mid -> deep` plus a root-level `leaf`, in the
    /// camelCase wire shape the shell sends.
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

    // --- WasmUndoStack bridge (FC-15) ---

    /// A `set-transform` op JSON to `(tx, ty)`, matching the camelCase wire shape
    /// the shell sends. Used to drive the JSON handshake end to end.
    fn set_transform_json(tx: f64, ty: f64) -> String {
        format!(
            r#"{{"kind":"set-transform","id":"r","transform":[[1,0,{tx}],[0,1,{ty}],[0,0,1]]}}"#
        )
    }

    /// A one-rect scene whose only object is at the identity transform, applied
    /// through the SAME wasm bridge the shell calls. Returns the scene JSON.
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
