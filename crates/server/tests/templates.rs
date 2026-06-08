//! Integration tests for the object-template HTTP surface
//! (`GET /api/templates`), driving the router in-process via
//! `tower::ServiceExt::oneshot` (no socket bind).
//!
//! Object templates are code-defined builtin recipes in
//! `shape_scene_core::object::templates`, so the catalog is read-only: there is
//! no user-template CRUD and no persistence (the legacy recipe `TemplateContract`
//! store was removed at OB-follow-up 1).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use shape_server::{build_router_with_mcp, CanvasRegistry, Config};
use tower::ServiceExt;

fn test_config() -> Config {
    Config {
        host: "127.0.0.1".to_string(),
        port: 0,
        client_dir: std::env::temp_dir().join("shape_server_test_no_client_dir"),
    }
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_returns_the_builtin_object_template_catalog() {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let app = build_router_with_mcp(&test_config(), canvases);

    let response = app
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

    // The served list is exactly the object catalog (same length + ids).
    let catalog = shape_scene_core::object::object_template_catalog();
    assert_eq!(templates.len(), catalog.len());
    assert!(!templates.is_empty(), "catalog is never empty");

    // Each served entry carries the picker metadata (id/label/category/description).
    for (served, expected) in templates.iter().zip(catalog.iter()) {
        assert_eq!(served["id"], expected.id);
        assert_eq!(served["label"], expected.label);
        assert!(served["description"].is_string());
        assert!(served["category"].is_string());
    }

    // The real builtins are present by id.
    let ids: Vec<&str> = templates.iter().map(|t| t["id"].as_str().unwrap()).collect();
    for want in ["decision_map", "todo_board", "idea_board", "wiki_note"] {
        assert!(ids.contains(&want), "missing {want}");
    }
}
