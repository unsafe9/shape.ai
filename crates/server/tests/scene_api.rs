//! OB4.1: the bespoke REST domain routes (`/api/groups` seed + export + artifact
//! download, `/api/comments`) are gone — those mutations are now Feature frames
//! over WS (see `tests/ws.rs`). What survives on HTTP is the thin canvas CRUD the
//! switch UI drives. These tests assert those response shapes through the
//! assembled router via `tower::ServiceExt::oneshot`.

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
        client_dir: std::env::temp_dir().join("shape_server_scene_api_test_no_client_dir"),
    }
}

fn router() -> axum::Router {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    build_router_with_mcp(&test_config(), canvases)
}

async fn send_json(app: &axum::Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn canvas_create_list_delete_round_trip_over_http() {
    let app = router();

    // Empty to start.
    let (status, body) = get_json(&app, "/api/canvases").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["canvases"].as_array().unwrap().len(), 0);

    // Create one.
    let (status, body) = send_json(&app, "POST", "/api/canvases", json!({ "title": "Alpha" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["canvas"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["canvas"]["title"], "Alpha");

    // List shows it.
    let (_status, body) = get_json(&app, "/api/canvases").await;
    let canvases = body["canvases"].as_array().unwrap();
    assert_eq!(canvases.len(), 1);
    assert_eq!(canvases[0]["title"], "Alpha");

    // Delete it.
    let (status, _body) = send_json(&app, "DELETE", &format!("/api/canvases/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);

    let (_status, body) = get_json(&app, "/api/canvases").await;
    assert_eq!(body["canvases"].as_array().unwrap().len(), 0, "deleted canvas gone from list");
}

#[tokio::test]
async fn delete_unknown_canvas_is_404() {
    let app = router();
    let (status, _body) = send_json(&app, "DELETE", "/api/canvases/nope", Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
