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
use crate::object::model::{Geometry, ObjectScene};
use crate::object::op::ObjectOp;
use crate::object::region::{OutlineDeriver, StubOutlineDeriver};
use crate::object::templates::build_template as build_template_pure;
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
