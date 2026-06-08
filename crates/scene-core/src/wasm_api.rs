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

use serde::Serialize;
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

use crate::object::apply::apply_object_op as apply_object_op_pure;
use crate::object::commands::object_command_catalog_json;
use crate::object::drawing::{Brush, DrawingSession};
use crate::object::model::{Geometry, ObjectScene};
use crate::object::op::ObjectOp;
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
