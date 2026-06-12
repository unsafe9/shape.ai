//! Feature channel handlers: the request/response RPC that replaces the bespoke
//! REST surface. Mutating requests lower to [`ObjectOp`]s and push through the
//! single op-apply path (`apply_object_op` on the passed [`ObjectScene`]), so the
//! channel never grows a second way to mutate the scene; reads reply directly.
//!
//! Time, seq, and id allocation are injected through [`FeatureCtx`], so the
//! handler holds no clock/rng/IO and the same call is deterministic under a fixed
//! ctx. Pointer-width-agnostic: `seq`/`revision` ride the wire as `u64`.

use shape_scene_core::object::{
    apply_object_op, ApplyError, FeatureRequest, FeatureResponse, ObjectOp, ObjectScene,
    template_to_ops,
};

/// Injected effects so the handler stays pure (mutates only the given
/// [`ObjectScene`] through `apply_object_op`); the actor owns clock/seq/ids.
pub struct FeatureCtx<'a> {
    /// RFC-3339 wall clock; a mutating Feature stamps `scene.updated_at` after apply.
    pub now: &'a dyn Fn() -> String,
    /// Current journal seq, echoed verbatim in `CanvasSwitched`.
    pub seq: u64,
    /// Current scene revision (`scene_version`), echoed on switch.
    pub revision: u64,
}

/// Lower a [`FeatureRequest`] to [`ObjectOp`]s, apply through the single op-apply
/// path, and return `(applied ops, response)`. The actor seam re-drives the
/// applied ops to capture inverses; this layer keeps lowering pure and owns no
/// journal. Reads (`CanvasSwitch`, `ExportRequest`) yield no op.
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
            // Recipe objects already carry ids/orders (the shell allocated them).
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
            scope_ids,
            export_type,
            request_id,
            ..
        } => {
            // Read-only: render the scoped objects + anchor connections into the
            // digest. Unknown formats are a FeatureError carrying the request_id.
            if !is_supported_export(&export_type) {
                return (
                    Vec::new(),
                    FeatureResponse::FeatureError {
                        request_id: Some(request_id),
                        message: format!("unsupported export type: {export_type}"),
                    },
                );
            }
            let artifact_ref = crate::object_mcp::export(scene, &scope_ids, &export_type);
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

/// Apply lowered ops in order. On failure the partial mutations stay (matching
/// `apply_object_op`'s in-place semantics); the caller turns the error into a
/// `FeatureError`. The inverse is discarded here — the actor seam re-derives it
/// for journaling.
fn apply_lowered(scene: &mut ObjectScene, ops: Vec<ObjectOp>) -> Result<Vec<ObjectOp>, ApplyError> {
    let mut applied = Vec::with_capacity(ops.len());
    for op in ops {
        let _inverse = apply_object_op(scene, op.clone())?;
        applied.push(op);
    }
    Ok(applied)
}

fn is_supported_export(export_type: &str) -> bool {
    matches!(export_type, "mermaid" | "digest")
}

fn export_content_type(export_type: &str) -> &'static str {
    match export_type {
        "mermaid" => "text/markdown",
        _ => "text/plain",
    }
}

fn feature_error(request_id: Option<String>, e: ApplyError) -> FeatureResponse {
    FeatureResponse::FeatureError {
        request_id,
        message: e.to_string(),
    }
}

/// A Feature request/response is a single internally-tagged JSON frame on the
/// reliable_ordered channel.
pub fn decode_feature(json: &str) -> Result<FeatureRequest, serde_json::Error> {
    serde_json::from_str(json)
}

/// Serialization of a well-formed response is infallible.
pub fn encode_feature_response(resp: &FeatureResponse) -> String {
    serde_json::to_string(resp).expect("feature response serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::object::{
        Anchor, Comment, FillRule, Geometry, LocalPoint, Object, PathNode, SubPath, Text, TextAlign,
        TextRun, TextVAlign,
    };

    fn labeled_rect(id: &str, order: &str, label: &str) -> Object {
        let mut obj = Object::new(id, order, rect_geometry());
        obj.text = Some(Text {
            runs: vec![TextRun {
                text: label.to_string(),
                color: None,
                size: None,
                bold: false,
                italic: false,
                font: None,
            }],
            align: TextAlign::default(),
            valign: TextVAlign::default(),
        });
        obj
    }

    /// Open 2-node connector anchored from `a` (node 0) to `b` (node 1).
    fn connector(id: &str, order: &str, a: &str, b: &str) -> Object {
        let mut obj = Object::new(
            id,
            order,
            Geometry::from_subpaths(
                vec![SubPath {
                    closed: false,
                    nodes: vec![PathNode::corner(0, 0), PathNode::corner(320, 0)],
                }],
                FillRule::NonZero,
            ),
        );
        obj.anchors = vec![
            Anchor { node_index: 0, target: a.to_string(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: b.to_string(), at: LocalPoint { x: 0, y: 0 } },
        ];
        obj
    }

    /// A closed rect at (0,0)-(80,40) in quantized units.
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

    fn fixed_clock() -> impl Fn() -> String {
        || "1970-01-01T00:00:00Z".to_string()
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
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
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

        assert_eq!(scene.get("rect-1").unwrap().comments, vec![comment]);
        assert_eq!(scene.updated_at, "1970-01-01T00:00:00Z");
        assert_eq!(applied.len(), 1);
        assert!(matches!(applied[0], ObjectOp::AddComment { .. }));
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
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
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
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
        };
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

        assert_eq!(scene.objects.len(), 2);
        assert!(scene.get("tpl-a").is_some());
        assert!(scene.get("tpl-b").is_some());
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
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
        };
        let req = FeatureRequest::CanvasSwitch {
            canvas_id: "cv-2".into(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

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
    fn export_request_renders_connected_scope_into_artifact() {
        let mut scene = ObjectScene::default();
        for op in [
            ObjectOp::InsertObject { object: labeled_rect("src", "a0", "Source") },
            ObjectOp::InsertObject { object: labeled_rect("dst", "a1", "Dest") },
            ObjectOp::InsertObject { object: connector("edge", "a2", "src", "dst") },
        ] {
            apply_object_op(&mut scene, op).expect("seed insert");
        }
        let clock = fixed_clock();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
        };
        let req = FeatureRequest::ExportRequest {
            canvas_id: "cv-1".into(),
            scope_ids: vec![],
            export_type: "digest".into(),
            request_id: "req-9".into(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        assert!(applied.is_empty());
        match resp {
            FeatureResponse::ExportReady {
                request_id,
                artifact_ref,
                content_type,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(content_type, "text/plain");
                assert!(!artifact_ref.is_empty(), "export content is non-empty");
                assert!(artifact_ref.contains("Source"), "mentions src label: {artifact_ref}");
                assert!(artifact_ref.contains("Dest"), "mentions dst label: {artifact_ref}");
                assert!(artifact_ref.contains("src -> dst"), "has the edge: {artifact_ref}");
            }
            other => panic!("expected ExportReady, got {other:?}"),
        }
    }

    #[test]
    fn export_request_mermaid_is_markdown() {
        let mut scene = ObjectScene::default();
        for op in [
            ObjectOp::InsertObject { object: labeled_rect("src", "a0", "Source") },
            ObjectOp::InsertObject { object: labeled_rect("dst", "a1", "Dest") },
            ObjectOp::InsertObject { object: connector("edge", "a2", "src", "dst") },
        ] {
            apply_object_op(&mut scene, op).expect("seed insert");
        }
        let clock = fixed_clock();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
        };
        let req = FeatureRequest::ExportRequest {
            canvas_id: "cv-1".into(),
            scope_ids: vec![],
            export_type: "mermaid".into(),
            request_id: "req-m".into(),
        };

        let (applied, resp) = handle_feature(req, &mut scene, &mut ctx);

        assert!(applied.is_empty());
        match resp {
            FeatureResponse::ExportReady { artifact_ref, content_type, .. } => {
                assert_eq!(content_type, "text/markdown");
                assert!(artifact_ref.starts_with("flowchart LR"));
                assert!(artifact_ref.contains("src --> dst"));
            }
            other => panic!("expected ExportReady, got {other:?}"),
        }
    }

    #[test]
    fn export_request_unsupported_yields_feature_error() {
        let mut scene = ObjectScene::default();
        let clock = fixed_clock();
        let mut ctx = FeatureCtx {
            now: &clock,
            seq: 7,
            revision: 42,
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
            // Geometry serializes only its path-string `d`; `ensure_parsed`
            // rebuilds the `subpaths` mirror, so hydrate before comparing.
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
