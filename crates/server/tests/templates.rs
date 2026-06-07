//! Integration tests for the CC3.2 template HTTP surface
//! (`GET/POST /api/templates`, `DELETE /api/templates/:id`), driving the router
//! in-process via `tower::ServiceExt::oneshot` (no socket bind).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use shape_server::{build_router_with_mcp, CanvasRegistry, Config};
use tower::ServiceExt;

fn test_config() -> Config {
    Config {
        host: "127.0.0.1".to_string(),
        port: 0,
        client_dir: std::env::temp_dir().join("shape_server_test_no_client_dir"),
    }
}

/// A minimal user TemplateContract JSON body (camelCase wire shape).
fn user_template_body(id: &str) -> Value {
    json!({
        "metadata": {
            "id": id,
            "title": format!("User {id}"),
            "description": "",
            "category": "general",
            "templateKind": "user",
        },
        "recipe": { "frames": [], "shapes": [], "edges": [] },
        "layout": {},
        "exports": { "allowed": [] },
        "tags": { "suggested": [] },
    })
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_seeds_builtins_post_and_delete_user_template() {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    // build_router_with_mcp seeds the builtins on assembly.
    let app = build_router_with_mcp(&test_config(), canvases);

    // GET lists the seeded builtins.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/templates")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let templates = body["templates"].as_array().unwrap();
    let builtin_count = shape_scene_core::registry().len();
    assert_eq!(templates.len(), builtin_count, "builtins seeded");

    // POST a user template.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/templates")
                .header("content-type", "application/json")
                .body(Body::from(user_template_body("my-tpl").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // GET now lists builtins + the user template.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/templates")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response).await;
    let templates = body["templates"].as_array().unwrap();
    assert_eq!(templates.len(), builtin_count + 1);
    assert!(templates
        .iter()
        .any(|t| t["metadata"]["id"] == "my-tpl"));

    // DELETE the user template.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/templates/my-tpl")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // DELETE again on the now-absent id is a 404 (no template Record existed).
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/templates/my-tpl")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // GET is back to just the builtins.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/templates")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response).await;
    assert_eq!(body["templates"].as_array().unwrap().len(), builtin_count);
}

#[tokio::test]
async fn deleting_a_builtin_via_http_tombstones_it() {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let app = build_router_with_mcp(&test_config(), canvases.clone());

    let victim = shape_scene_core::registry()[0].metadata.id.clone();
    let builtin_count = shape_scene_core::registry().len();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/templates/{victim}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Re-seeding does not bring the deleted builtin back (tombstone holds).
    let written = canvases.seed_templates().unwrap();
    assert_eq!(written, 0, "guard already set; re-seed is a no-op");
    let listed = canvases.list_templates();
    assert_eq!(listed.len(), builtin_count - 1);
    assert!(!listed.iter().any(|t| t.metadata.id == victim));
}
