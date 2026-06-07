//! Integration tests for the residual server-side scene routes (`/api/groups`
//! seed + export + artifact download, `/api/comments`) that survive the MG-7
//! client cutover because they have no scene-core op. Driven through the assembled
//! axum router (`build_router_with_mcp`) via `tower::ServiceExt::oneshot`; these
//! assert the response shapes the Svelte shell consumes.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use shape_server::{build_router_with_mcp, CanvasRegistry, ClientRegistry, Config};
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
    let clients = ClientRegistry::new();
    build_router_with_mcp(&test_config(), canvases, clients)
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

/// Create a group via POST /api/groups and return its id.
async fn create_group(app: &axum::Router, prompt: &str) -> (Value, String) {
    let (status, body) = send_json(app, "POST", "/api/groups", json!({ "prompt": prompt })).await;
    assert_eq!(status, StatusCode::CREATED, "create group: {body}");
    let id = body["group"]["id"].as_str().unwrap().to_string();
    (body, id)
}

#[tokio::test]
async fn create_group_seeds_ten_nodes_nine_edges() {
    let app = router();
    let (body, group_id) = create_group(&app, "Choose a database").await;

    assert!(group_id.starts_with("group-"));
    assert_eq!(body["group"]["title"], "Choose a database");
    assert_eq!(body["message"], "Created a group on the infinite scene canvas.");

    let scene = &body["scene"];
    assert_eq!(scene["groups"].as_array().unwrap().len(), 1);
    assert_eq!(
        scene["nodes"].as_array().unwrap().len(),
        10,
        "seed produces 10 nodes"
    );
    assert_eq!(
        scene["edges"].as_array().unwrap().len(),
        9,
        "seed produces 9 edges"
    );

    // The proposition node's title is the prompt-derived title.
    let proposition = scene["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"].as_str().unwrap().ends_with("-n-proposition"))
        .unwrap();
    assert_eq!(proposition["title"], "Choose a database");
    assert_eq!(proposition["type"], "proposition");
}

#[tokio::test]
async fn second_group_is_offset_so_it_does_not_overlap() {
    let app = router();
    let (_b1, _id1) = create_group(&app, "First").await;
    let (b2, _id2) = create_group(&app, "Second").await;
    let scene = &b2["scene"];
    let groups = scene["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    // Two distinct top-level bounds.x: the second is packed onto a new grid cell.
    let xs: Vec<f64> = groups.iter().map(|g| g["bounds"]["x"].as_f64().unwrap()).collect();
    assert_ne!(xs[0], xs[1], "second group is offset onto a free grid cell");
}

#[tokio::test]
async fn comment_add_and_update() {
    let app = router();
    let (_b, group_id) = create_group(&app, "Commentable").await;

    let (status, body) = send_json(
        &app,
        "POST",
        "/api/comments",
        json!({ "target": { "kind": "group", "id": group_id }, "body": "looks good" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["comment"]["body"], "looks good");
    assert_eq!(body["comment"]["resolved"], false);
    assert_eq!(body["scene"]["comments"].as_array().unwrap().len(), 1);
    let comment_id = body["comment"]["id"].as_str().unwrap().to_string();

    // Update: resolve it + change body.
    let (status, body) = send_json(
        &app,
        "PATCH",
        &format!("/api/comments/{comment_id}"),
        json!({ "body": "resolved now", "resolved": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["comment"]["body"], "resolved now");
    assert_eq!(body["comment"]["resolved"], true);
    assert_eq!(body["scene"]["comments"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn export_madr_and_mermaid_shapes() {
    let app = router();
    let (_b, group_id) = create_group(&app, "Export me").await;

    // MADR export.
    let (status, body) = send_json(
        &app,
        "POST",
        &format!("/api/groups/{group_id}/export"),
        json!({ "type": "madr" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["preview"]["type"], "madr");
    assert_eq!(body["preview"]["contentType"], "text/markdown; charset=utf-8");
    let madr = body["preview"]["content"].as_str().unwrap();
    assert!(!madr.is_empty());
    assert!(madr.contains("## Context and Problem Statement"));
    assert!(madr.contains("## Decision Outcome"));
    assert!(madr.contains("## More Information"));
    // Artifact persisted onto the scene.
    assert_eq!(body["scene"]["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(body["artifact"]["type"], "madr");
    let artifact_id = body["artifact"]["id"].as_str().unwrap().to_string();

    // Mermaid export.
    let (status, body) = send_json(
        &app,
        "POST",
        &format!("/api/groups/{group_id}/export"),
        json!({ "type": "mermaid" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["preview"]["type"], "mermaid");
    assert_eq!(body["preview"]["contentType"], "text/plain; charset=utf-8");
    let mermaid = body["preview"]["content"].as_str().unwrap();
    assert!(mermaid.starts_with("flowchart LR"), "got: {mermaid}");
    assert!(mermaid.contains("-->"));

    // Download the persisted MADR artifact file.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/groups/{group_id}/artifacts/{artifact_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let content = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(content.contains("## Context and Problem Statement"));
}

#[tokio::test]
async fn export_image_prompt_includes_image_prompt_field() {
    let app = router();
    let (_b, group_id) = create_group(&app, "Picture").await;
    let (status, body) = send_json(
        &app,
        "POST",
        &format!("/api/groups/{group_id}/export"),
        json!({ "type": "image_prompt" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["preview"]["type"], "image_prompt");
    let prompt = body["preview"]["imagePrompt"].as_str().unwrap();
    assert!(prompt.contains("architecture decision diagram"));
    assert_eq!(body["preview"]["content"], body["preview"]["imagePrompt"]);
}

#[tokio::test]
async fn export_unknown_group_is_404() {
    let app = router();
    let (status, _body) = send_json(
        &app,
        "POST",
        "/api/groups/group-missing/export",
        json!({ "type": "madr" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
