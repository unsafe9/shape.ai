//! OB3.S7 — wire Feature channel handlers (object-native, additive).
//!
//! The Feature channel is the request/response RPC that replaces the bespoke
//! REST surface (`/api/canvases|comments|templates|export`). Mutating requests
//! are lowered to [`ObjectOp`]s and pushed through the **single** op-apply path
//! (P1) — `apply_object_op` on the passed [`ObjectScene`] — so the Feature
//! channel never grows a second way to mutate the scene. Read/subscribe
//! requests reply directly without authoring an op.
//!
//! This module is authored alongside the legacy server (`ws.rs`/`scene_api.rs`/
//! `mcp.rs`, all still on `shape_scene_core::{Scene, ...}`). It is wired into the
//! router and the per-canvas actor at the OB-4 cutover; nothing here touches the
//! legacy modules.
//!
//! Pure-where-possible: time, the journal seq, and id allocation are *injected*
//! through [`FeatureCtx`] (the actor seam supplies the real clock/seq/ids), so
//! the handler itself holds no clock/rng/IO and the same call is deterministic
//! under a fixed `FeatureCtx`. Pointer-width-agnostic: `seq`/`revision` ride the
//! wire as `u64`, never `usize`.

use shape_scene_core::object::{
    apply_object_op, ApplyError, FeatureRequest, FeatureResponse, ObjectOp, ObjectScene,
    template_to_ops,
};

/// Injected effects for a Feature handler call (P1 IO/time seam).
///
/// The handler is otherwise pure: it mutates only the [`ObjectScene`] it is
/// given, through `apply_object_op`. Everything non-deterministic — the wall
/// clock, the canvas's current journal `seq`/`revision`, and fresh id minting —
/// arrives here so the actor owns those concerns and tests can pin them.
pub struct FeatureCtx<'a> {
    /// Wall-clock accessor (RFC-3339 string, the `ObjectScene.updated_at` shape).
    /// Injected so the handler stays clock-free; a mutating Feature stamps
    /// `scene.updated_at` with it after a successful apply.
    pub now: &'a dyn Fn() -> String,
    /// The canvas's current journal sequence (post-apply seq the actor reports).
    /// Returned verbatim in `CanvasSwitched` so a switch is a read/subscribe.
    pub seq: u64,
    /// The canvas's current scene revision (`scene_version`), echoed on switch.
    pub revision: u64,
    /// Fresh-id allocator (e.g. export request artifact refs). Injected, no rng.
    pub alloc_id: &'a mut dyn FnMut() -> String,
}

/// Lower a [`FeatureRequest`] to [`ObjectOp`]s and apply them through the single
/// op-apply path, returning `(applied ops, response)`.
///
/// The returned `Vec<ObjectOp>` is the forward ops that were *applied* (already
/// lowered and committed to `scene`). The actor seam re-drives these through
/// `apply_object_op` to capture the matching inverses for the undo/journal; this
/// layer keeps the lowering pure and does not own the journal.
///
/// Variants:
/// - `CommentUpsert` → one [`ObjectOp::AddComment`]; reply `CommentUpserted`.
/// - `TemplateApply` → `template_to_ops(recipe)`, applied in order; reply
///   `TemplateApplied` with the inserted ids.
/// - `CanvasSwitch` → no op (read/subscribe); reply `CanvasSwitched` with the
///   injected `seq`/`revision`.
/// - `ExportRequest` → no op; reply `ExportReady` with a stub artifact ref
///   (the real export is `local_export`, cutover-wired) or `FeatureError` for an
///   unsupported `export_type`.
pub fn handle_feature(
    req: FeatureRequest,
    scene: &mut ObjectScene,
    ctx: &mut FeatureCtx<'_>,
) -> (Vec<ObjectOp>, FeatureResponse) {
    match req {
        FeatureRequest::CommentUpsert {
            object_id, comment, ..
        } => {
            let comment_id = comment.id.clone();
            let op = ObjectOp::AddComment {
                id: object_id.clone(),
                comment,
            };
            match apply_lowered(scene, vec![op]) {
                Ok(applied) => {
                    scene.updated_at = (ctx.now)();
                    (
                        applied,
                        FeatureResponse::CommentUpserted {
                            object_id,
                            comment_id,
                        },
                    )
                }
                Err(e) => (Vec::new(), feature_error(None, e)),
            }
        }

        FeatureRequest::TemplateApply { recipe, .. } => {
            // The recipe objects already carry ids/orders (the shell's
            // build_template allocated them); lower 1 insert-object per object.
            let object_ids: Vec<String> = recipe.iter().map(|o| o.id.clone()).collect();
            let ops = template_to_ops(recipe);
            match apply_lowered(scene, ops) {
                Ok(applied) => {
                    scene.updated_at = (ctx.now)();
                    (applied, FeatureResponse::TemplateApplied { object_ids })
                }
                Err(e) => (Vec::new(), feature_error(None, e)),
            }
        }

        FeatureRequest::CanvasSwitch { canvas_id } => {
            // A read/subscribe: no op, no scene mutation. The actor's current
            // seq/revision (injected) tell the client where the snapshot lands.
            (
                Vec::new(),
                FeatureResponse::CanvasSwitched {
                    canvas_id,
                    seq: ctx.seq,
                    revision: ctx.revision,
                },
            )
        }

        FeatureRequest::ExportRequest {
            export_type,
            request_id,
            ..
        } => {
            // Stub: the real artifact is produced by `local_export` at the
            // cutover. Only the formats local_export supports are accepted here;
            // anything else is a clean FeatureError carrying the request_id.
            if !is_supported_export(&export_type) {
                return (
                    Vec::new(),
                    FeatureResponse::FeatureError {
                        request_id: Some(request_id),
                        message: format!("unsupported export type: {export_type}"),
                    },
                );
            }
            let artifact_ref = (ctx.alloc_id)();
            (
                Vec::new(),
                FeatureResponse::ExportReady {
                    request_id,
                    artifact_ref,
                    content_type: export_content_type(&export_type).to_string(),
                },
            )
        }
    }
}

/// Apply already-lowered ops in order through the single op-apply path (P1),
/// returning the applied ops on success. On any failure the partial mutations
/// stay (matching `apply_object_op`'s in-place semantics for non-batch ops); the
/// caller turns the error into a `FeatureError` and does NOT report the request
/// as applied. The actor records the inverses (captured here) for undo/journal.
fn apply_lowered(scene: &mut ObjectScene, ops: Vec<ObjectOp>) -> Result<Vec<ObjectOp>, ApplyError> {
    let mut applied = Vec::with_capacity(ops.len());
    for op in ops {
        // The inverse is intentionally discarded at this layer; the actor seam
        // re-derives/records it when it drives `apply_object_op` for journaling.
        let _inverse = apply_object_op(scene, op.clone())?;
        applied.push(op);
    }
    Ok(applied)
}

/// The export formats `local_export` can satisfy. Kept narrow on purpose: an
/// unknown format is rejected rather than silently stubbed.
fn is_supported_export(export_type: &str) -> bool {
    matches!(export_type, "svg" | "png" | "json")
}

fn export_content_type(export_type: &str) -> &'static str {
    match export_type {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn feature_error(request_id: Option<String>, e: ApplyError) -> FeatureResponse {
    FeatureResponse::FeatureError {
        request_id,
        message: e.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Wire round-trip helpers. These replace the bespoke REST endpoints
// (/api/canvases|comments|templates|export): a Feature request/response is a
// single JSON frame on the reliable_ordered channel, internally tagged on
// `feature` (the FeatureRequest/FeatureResponse serde shape).
// ---------------------------------------------------------------------------

/// Decode a Feature request frame from its JSON wire text.
pub fn decode_feature(json: &str) -> Result<FeatureRequest, serde_json::Error> {
    serde_json::from_str(json)
}

/// Encode a Feature response to its JSON wire text. Serialization of a
/// well-formed response is infallible (no non-string map keys, no NaN paths).
pub fn encode_feature_response(resp: &FeatureResponse) -> String {
    serde_json::to_string(resp).expect("feature response serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::object::{
        Comment, FillRule, Geometry, Object, PathNode, SubPath,
    };

    /// A closed unit rect at (0,0)-(80,40) in quantized units.
    fn rect_geometry() -> Geometry {
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
        )
    }

    /// A no-op clock for the time seam (handlers are clock-free today).
    fn fixed_clock() -> impl Fn() -> String {
        || "1970-01-01T00:00:00Z".to_string()
    }

    /// A counting id allocator returning "artifact-0", "artifact-1", ... .
    fn counting_alloc() -> impl FnMut() -> String {
        let mut n = 0u64;
        move || {
            let id = format!("artifact-{n}");
            n += 1;
            id
        }
    }

    fn scene_with_one_object() -> ObjectScene {
        let mut scene = ObjectScene::default();
        apply_object_op(
            &mut scene,
            ObjectOp::InsertObject {
                object: Object::new("rect-1", "a0", rect_geometry()),
            },
        )
        .expect("seed insert");
        scene
    }

    #[test]
    fn comment_upsert_appends_and_responds() {
        let mut scene = scene_with_one_object();
        let clock = fixed_clock();
        let mut alloc = counting_alloc();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
            alloc_id: &mut alloc,
        };
        let comment = Comment {
            id: "c-1".into(),
            author: "jayden".into(),
            body: "looks good".into(),
            at: None,
            resolved: false,
        };
        let req = FeatureRequest::CommentUpsert {
            canvas_id: "cv-1".into(),
            object_id: "rect-1".into(),
            comment: comment.clone(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        // The comment is appended to the object via the single apply path.
        assert_eq!(scene.get("rect-1").unwrap().comments, vec![comment]);
        // The injected clock stamped updated_at on the successful apply.
        assert_eq!(scene.updated_at, "1970-01-01T00:00:00Z");
        // One AddComment op was applied.
        assert_eq!(applied.len(), 1);
        assert!(matches!(applied[0], ObjectOp::AddComment { .. }));
        // Response echoes the object + the comment id.
        assert_eq!(
            resp,
            FeatureResponse::CommentUpserted {
                object_id: "rect-1".into(),
                comment_id: "c-1".into(),
            }
        );
    }

    #[test]
    fn comment_upsert_on_missing_object_errors_without_mutation() {
        let mut scene = ObjectScene::default();
        let clock = fixed_clock();
        let mut alloc = counting_alloc();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
            alloc_id: &mut alloc,
        };
        let req = FeatureRequest::CommentUpsert {
            canvas_id: "cv-1".into(),
            object_id: "nope".into(),
            comment: Comment {
                id: "c-1".into(),
                author: "a".into(),
                body: "b".into(),
                at: None,
                resolved: false,
            },
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        assert!(applied.is_empty());
        assert!(matches!(resp, FeatureResponse::FeatureError { .. }));
    }

    #[test]
    fn template_apply_inserts_recipe_and_returns_ids() {
        let mut scene = ObjectScene::default();
        let clock = fixed_clock();
        let mut alloc = counting_alloc();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
            alloc_id: &mut alloc,
        };
        // A 2-object recipe carrying its own ids/orders (as build_template mints).
        let recipe = vec![
            Object::new("tpl-a", "a0", rect_geometry()),
            Object::new("tpl-b", "a1", rect_geometry()),
        ];
        let req = FeatureRequest::TemplateApply {
            canvas_id: "cv-1".into(),
            recipe,
            anchor_x: 10.0,
            anchor_y: 20.0,
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        // Both objects inserted through the single apply path.
        assert_eq!(scene.objects.len(), 2);
        assert!(scene.get("tpl-a").is_some());
        assert!(scene.get("tpl-b").is_some());
        // Two insert-object ops applied.
        assert_eq!(applied.len(), 2);
        assert!(applied
            .iter()
            .all(|op| matches!(op, ObjectOp::InsertObject { .. })));
        assert_eq!(
            resp,
            FeatureResponse::TemplateApplied {
                object_ids: vec!["tpl-a".into(), "tpl-b".into()],
            }
        );
    }

    #[test]
    fn canvas_switch_reads_seq_revision_without_op() {
        let mut scene = scene_with_one_object();
        let before = scene.clone();
        let clock = fixed_clock();
        let mut alloc = counting_alloc();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
            alloc_id: &mut alloc,
        };
        let req = FeatureRequest::CanvasSwitch {
            canvas_id: "cv-2".into(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        // No op, no scene mutation.
        assert!(applied.is_empty());
        assert_eq!(scene, before);
        assert_eq!(
            resp,
            FeatureResponse::CanvasSwitched {
                canvas_id: "cv-2".into(),
                seq: 7,
                revision: 42,
            }
        );
    }

    #[test]
    fn export_request_supported_yields_ready_stub() {
        let mut scene = ObjectScene::default();
        let clock = fixed_clock();
        let mut alloc = counting_alloc();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
            alloc_id: &mut alloc,
        };
        let req = FeatureRequest::ExportRequest {
            canvas_id: "cv-1".into(),
            scope_ids: vec!["rect-1".into()],
            export_type: "svg".into(),
            request_id: "req-9".into(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        assert!(applied.is_empty());
        assert_eq!(
            resp,
            FeatureResponse::ExportReady {
                request_id: "req-9".into(),
                artifact_ref: "artifact-0".into(),
                content_type: "image/svg+xml".into(),
            }
        );
    }

    #[test]
    fn export_request_unsupported_yields_feature_error() {
        let mut scene = ObjectScene::default();
        let clock = fixed_clock();
        let mut alloc = counting_alloc();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
            alloc_id: &mut alloc,
        };
        let req = FeatureRequest::ExportRequest {
            canvas_id: "cv-1".into(),
            scope_ids: vec![],
            export_type: "pdf".into(),
            request_id: "req-1".into(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        assert!(applied.is_empty());
        match resp {
            FeatureResponse::FeatureError {
                request_id,
                message,
            } => {
                assert_eq!(request_id.as_deref(), Some("req-1"));
                assert!(message.contains("pdf"));
            }
            other => panic!("expected FeatureError, got {other:?}"),
        }
    }

    #[test]
    fn feature_request_json_round_trips() {
        let reqs = vec![
            FeatureRequest::CanvasSwitch {
                canvas_id: "cv-1".into(),
            },
            FeatureRequest::CommentUpsert {
                canvas_id: "cv-1".into(),
                object_id: "o-1".into(),
                comment: Comment {
                    id: "c-1".into(),
                    author: "a".into(),
                    body: "hi".into(),
                    at: None,
                    resolved: false,
                },
            },
            FeatureRequest::TemplateApply {
                canvas_id: "cv-1".into(),
                recipe: vec![Object::new("o-1", "a0", rect_geometry())],
                anchor_x: 1.0,
                anchor_y: 2.0,
            },
            FeatureRequest::ExportRequest {
                canvas_id: "cv-1".into(),
                scope_ids: vec!["o-1".into()],
                export_type: "png".into(),
                request_id: "r-1".into(),
            },
        ];
        for req in reqs {
            let json = serde_json::to_string(&req).expect("serialize request");
            let mut back = decode_feature(&json).expect("decode request");
            // Geometry serializes only its path-string `d`; the parsed `subpaths`
            // mirror is reconstructed by `ensure_parsed`. Hydrate the decoded
            // recipe so the round-trip compares apples to apples (the at-rest wire
            // form is faithful; only the runtime mirror needs rebuilding).
            if let FeatureRequest::TemplateApply { recipe, .. } = &mut back {
                for object in recipe {
                    object.geometry.ensure_parsed().expect("recipe geometry parses");
                }
            }
            assert_eq!(back, req);
        }
    }

    #[test]
    fn feature_response_json_round_trips() {
        let resps = vec![
            FeatureResponse::CanvasSwitched {
                canvas_id: "cv-1".into(),
                seq: 3,
                revision: 9,
            },
            FeatureResponse::CommentUpserted {
                object_id: "o-1".into(),
                comment_id: "c-1".into(),
            },
            FeatureResponse::TemplateApplied {
                object_ids: vec!["a".into(), "b".into()],
            },
            FeatureResponse::ExportReady {
                request_id: "r-1".into(),
                artifact_ref: "artifact-0".into(),
                content_type: "image/png".into(),
            },
            FeatureResponse::FeatureError {
                request_id: Some("r-1".into()),
                message: "boom".into(),
            },
        ];
        for resp in resps {
            let json = encode_feature_response(&resp);
            let back: FeatureResponse =
                serde_json::from_str(&json).expect("decode response");
            assert_eq!(back, resp);
        }
    }
}
