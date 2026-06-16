//! Integration tests for `GET /api/templates`. Object templates are code-defined
//! builtin recipes, so the catalog is read-only — no CRUD, no persistence.

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

    let catalog = shape_scene_core::object::object_template_catalog();
    assert_eq!(templates.len(), catalog.len());
    assert!(!templates.is_empty(), "catalog is never empty");

    for (served, expected) in templates.iter().zip(catalog.iter()) {
        assert_eq!(served["id"], expected.id);
        assert_eq!(served["label"], expected.label);
        assert!(served["description"].is_string());
        assert!(served["category"].is_string());
    }

    let ids: Vec<&str> = templates.iter().map(|t| t["id"].as_str().unwrap()).collect();
    for want in ["decision_map", "todo_board", "idea_board", "wiki_note"] {
        assert!(ids.contains(&want), "missing {want}");
    }
}

/// `GET /api/extensions` serves the data-rep extension tool catalog, namespaced
/// `ext_<name>_<tool>` — the same self-describing set `/mcp` advertises. Falsifiable
/// against a dropped extension registration (the catalog would lose the group).
#[tokio::test]
async fn list_returns_the_namespaced_extension_tool_catalog() {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let app = build_router_with_mcp(&test_config(), canvases);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/extensions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    let tools = body["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in ["ext_kanban_add_card", "ext_diagram_connect"] {
        assert!(names.contains(&want), "missing extension tool {want} in {names:?}");
    }
    for tool in tools {
        assert!(tool["description"].is_string());
        assert_eq!(tool["schema"]["type"], "object");
    }
}
