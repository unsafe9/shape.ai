//! Integration tests for the HTTP surface, driving the router in-process via
//! `oneshot` (no socket bind).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use shape_server::{build_router, Config};
use tower::ServiceExt;

fn test_config() -> Config {
    // No client assets so static hosting is skipped and the API is exercised
    // deterministically.
    Config {
        host: "127.0.0.1".to_string(),
        port: 0,
        client_dir: std::env::temp_dir().join("shape_server_test_no_client_dir"),
    }
}

#[tokio::test]
async fn health_returns_ok_json() {
    let app = build_router(&test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["ok"], Value::Bool(true));
    assert_eq!(body["name"], Value::String("shape_server".to_string()));
}

#[tokio::test]
async fn ready_returns_200() {
    let app = build_router(&test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
