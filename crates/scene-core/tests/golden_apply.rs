//! Golden-vector equivalence test (MG0.2a).
//!
//! Loads cases generated from the canonical TS `renderPatch.ts` and asserts the
//! Rust port produces an equivalent output scene + identical error list. Numbers
//! are normalized to f64 (JS has no int/float distinction); comment ids are
//! masked where the TS source uses a non-deterministic `Date.now()` id.

use serde_json::Value;
use shape_scene_core::apply::{
    add_shape_scene_comment, apply_render_patch_to_shape_scene, update_shape_scene_group_tags,
};
use shape_scene_core::model::{Scene, SceneSelection};
use shape_scene_core::op::RenderScenePatch;

const CASES: &str = include_str!("golden/cases.json");

/// Recursively rewrite every JSON number as an f64 so `0` and `0.0` compare equal.
fn normalize(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if let Some(nn) = serde_json::Number::from_f64(f) {
                    *n = nn;
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        Value::Object(map) => map.values_mut().for_each(normalize),
        _ => {}
    }
}

/// Null out `comments[].id` (non-deterministic between TS and Rust).
fn mask_comment_ids(scene: &mut Value) {
    if let Some(comments) = scene.get_mut("comments").and_then(Value::as_array_mut) {
        for c in comments {
            if let Some(obj) = c.as_object_mut() {
                obj.insert("id".to_string(), Value::Null);
            }
        }
    }
}

fn str_field<'a>(case: &'a Value, key: &str) -> &'a str {
    case.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("case missing string field {key}"))
}

#[test]
fn golden_vectors_match_ts() {
    let cases: Vec<Value> = serde_json::from_str(CASES).expect("parse cases.json");
    assert!(!cases.is_empty(), "no golden cases");

    let mut checked = 0usize;
    for case in &cases {
        let name = str_field(case, "name");
        let now = str_field(case, "now");
        let kind = str_field(case, "fn");
        let scene: Scene =
            serde_json::from_value(case["scene"].clone()).unwrap_or_else(|e| panic!("[{name}] scene deser: {e}"));

        let (mut got_scene, got_errors): (Value, Vec<String>) = match kind {
            "apply" => {
                let patch: RenderScenePatch = serde_json::from_value(case["patch"].clone())
                    .unwrap_or_else(|e| panic!("[{name}] patch deser: {e}"));
                let result = apply_render_patch_to_shape_scene(&scene, &patch, now, None);
                (serde_json::to_value(&result.scene).unwrap(), result.errors)
            }
            "updateGroupTags" => {
                let group_id = str_field(case, "groupId");
                let tag_ids: Vec<String> =
                    serde_json::from_value(case["tagIds"].clone()).unwrap();
                let result = update_shape_scene_group_tags(&scene, group_id, &tag_ids, now);
                (serde_json::to_value(&result.scene).unwrap(), result.errors)
            }
            "addComment" => {
                let target: SceneSelection =
                    serde_json::from_value(case["target"].clone()).unwrap();
                let body = str_field(case, "body");
                let result = add_shape_scene_comment(&scene, &target, body, now);
                (serde_json::to_value(&result.scene).unwrap(), result.errors)
            }
            other => panic!("[{name}] unknown fn {other}"),
        };

        let mut want_scene = case["expected"]["scene"].clone();
        let want_errors: Vec<String> =
            serde_json::from_value(case["expected"]["errors"].clone()).unwrap();

        if case.get("maskCommentIds").and_then(Value::as_bool) == Some(true) {
            mask_comment_ids(&mut got_scene);
            mask_comment_ids(&mut want_scene);
        }

        normalize(&mut got_scene);
        normalize(&mut want_scene);

        assert_eq!(got_errors, want_errors, "[{name}] errors mismatch");
        if got_scene != want_scene {
            let got = serde_json::to_string_pretty(&got_scene).unwrap();
            let want = serde_json::to_string_pretty(&want_scene).unwrap();
            panic!("[{name}] scene mismatch\n--- got ---\n{got}\n--- want ---\n{want}");
        }
        checked += 1;
    }
    eprintln!("golden: {checked} cases matched");
}
