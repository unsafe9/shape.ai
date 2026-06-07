//! MG2.3 integration tests: exercise the MCP tool surface through the rmcp tool
//! handler fns (the same fns the streamable-HTTP transport dispatches) and the
//! companion-dock HTTP endpoints.
//!
//! The transport itself is rmcp's; these tests drive the underlying handlers
//! directly (no socket, no JSON-RPC framing) plus the dock HTTP routes via
//! `tower::ServiceExt::oneshot`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{json, Value};
use shape_server::mcp::{CreateGroupArgs, GetClientTraceArgs, PatchSceneArgs, QuerySceneArgs};
use shape_server::{build_router_with_mcp, CanvasRegistry, ClientRegistry, Config, SceneMcp};
use tower::ServiceExt;

fn test_config() -> Config {
    Config {
        host: "127.0.0.1".to_string(),
        port: 0,
        client_dir: std::env::temp_dir().join("shape_server_mcp_test_no_client_dir"),
    }
}

/// Build a SceneMcp over a fresh in-memory canvas registry + client registry.
fn mcp_instance() -> (SceneMcp, ClientRegistry) {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let clients = ClientRegistry::new();
    let mcp = SceneMcp::new(canvases, clients.clone(), "test-client");
    (mcp, clients)
}

/// Extract the JSON text from a tool result's single text content block.
fn result_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content block");
    serde_json::from_str(&text).expect("tool result is JSON")
}

#[test]
fn tool_router_lists_all_eleven_tools() {
    let tools = SceneMcp::tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tools.len() >= 10,
        "expected at least 10 tools, got {}: {names:?}",
        tools.len()
    );
    for expected in [
        "query_scene",
        "list_groups",
        "get_group",
        "create_group",
        "patch_scene",
        "create_tag",
        "update_group_tags",
        "set_selection",
        "add_comment",
        "export_group",
        "get_client_trace",
    ] {
        assert!(names.contains(&expected), "missing tool {expected} in {names:?}");
    }
    // Every tool must advertise an input schema for tools/list.
    for tool in &tools {
        assert!(
            !tool.input_schema.is_empty(),
            "tool {} has no input schema",
            tool.name
        );
    }
}

#[tokio::test]
async fn patch_scene_create_group_changes_the_canvas() {
    let (mcp, _clients) = mcp_instance();

    // Scene starts empty.
    let before = mcp
        .query_scene(Parameters(QuerySceneArgs::default()))
        .await
        .unwrap();
    let before = result_json(&before);
    assert_eq!(before["scene"]["groups"].as_array().unwrap().len(), 0);

    // patch_scene with a create-group RenderScenePatch.
    let patch = json!({
        "kind": "create-group",
        "group": {
            "id": "g-mcp",
            "title": "From MCP",
            "summary": "",
            "bounds": { "x": 0.0, "y": 0.0, "width": 400.0, "height": 300.0 },
            "tagIds": [],
            "zIndex": 0.0,
            "styleKey": ""
        }
    });
    let res = mcp
        .patch_scene(Parameters(PatchSceneArgs {
            patch,
            canvas_id: None,
        }))
        .await
        .expect("patch_scene applies");
    let res = result_json(&res);
    let groups = res["scene"]["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "create-group added a group");
    assert_eq!(groups[0]["id"], "g-mcp");
    assert_eq!(groups[0]["title"], "From MCP");

    // The change is durable on the canvas: a fresh query sees it.
    let after = mcp
        .query_scene(Parameters(QuerySceneArgs::default()))
        .await
        .unwrap();
    let after = result_json(&after);
    assert_eq!(after["scene"]["groups"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn patch_scene_rejects_invalid_patch_with_error() {
    let (mcp, _clients) = mcp_instance();
    // create-card against a missing group is rejected by scene-core.
    let patch = json!({
        "kind": "create-card",
        "card": {
            "id": "n1",
            "groupId": "missing",
            "title": "x",
            "bounds": { "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 }
        }
    });
    let err = mcp
        .patch_scene(Parameters(PatchSceneArgs {
            patch,
            canvas_id: None,
        }))
        .await
        .expect_err("invalid patch is an error");
    assert!(
        err.message.contains("Unknown group id"),
        "got {:?}",
        err.message
    );
}

#[tokio::test]
async fn create_group_then_get_group_returns_digest() {
    let (mcp, _clients) = mcp_instance();
    let created = mcp
        .create_group(Parameters(CreateGroupArgs {
            prompt: "Choose a database".to_string(),
            title: None,
            parent_group_id: None,
            tag_ids: None,
            canvas_id: None,
        }))
        .await
        .expect("create_group");
    let created = result_json(&created);
    let group_id = created["group"]["id"].as_str().unwrap().to_string();
    assert!(group_id.starts_with("group-"));
    assert_eq!(created["group"]["title"], "Choose a database");

    let detail = mcp
        .get_group(Parameters(shape_server::mcp::GetGroupArgs {
            group_id: group_id.clone(),
            canvas_id: None,
        }))
        .await
        .expect("get_group");
    let detail = result_json(&detail);
    assert_eq!(detail["group"]["id"], group_id);
    // Digest is the scene-core text digest (empty group => placeholders).
    assert!(detail["digest"].as_str().unwrap().contains("Nodes:"));
}

#[tokio::test]
async fn create_tag_update_group_tags_and_export() {
    let (mcp, _clients) = mcp_instance();

    // Make a group + a tag, attach the tag, then export.
    let g = mcp
        .create_group(Parameters(CreateGroupArgs {
            prompt: "Architecture".to_string(),
            title: Some("Arch".to_string()),
            parent_group_id: None,
            tag_ids: None,
            canvas_id: None,
        }))
        .await
        .unwrap();
    let group_id = result_json(&g)["group"]["id"].as_str().unwrap().to_string();

    let t = mcp
        .create_tag(Parameters(shape_server::mcp::CreateTagArgs {
            name: "Backend".to_string(),
            color: "#3d82e0".to_string(),
            description: None,
            canvas_id: None,
        }))
        .await
        .unwrap();
    let tag_id = result_json(&t)["tag"]["id"].as_str().unwrap().to_string();
    assert!(tag_id.starts_with("tag-backend-"));

    let updated = mcp
        .update_group_tags(Parameters(shape_server::mcp::UpdateGroupTagsArgs {
            group_id: group_id.clone(),
            tag_ids: vec![tag_id.clone()],
            canvas_id: None,
        }))
        .await
        .expect("update_group_tags");
    let updated = result_json(&updated);
    let tag_ids = updated["group"]["tagIds"].as_array().unwrap();
    assert_eq!(tag_ids.len(), 1);
    assert_eq!(tag_ids[0], tag_id);

    // Export a couple of formats; content is returned inline.
    let exported = mcp
        .export_group(Parameters(shape_server::mcp::ExportGroupArgs {
            group_id: group_id.clone(),
            r#type: Some("mermaid".to_string()),
            types: Some(vec!["madr".to_string()]),
            canvas_id: None,
        }))
        .await
        .expect("export_group");
    let exported = result_json(&exported);
    let exports = exported["exports"].as_array().unwrap();
    assert_eq!(exports.len(), 2);
    assert_eq!(exports[0]["type"], "mermaid");
    assert!(exports[0]["content"].as_str().unwrap().contains("flowchart LR"));
}

#[tokio::test]
async fn add_comment_attaches_to_a_group() {
    let (mcp, _clients) = mcp_instance();
    let g = mcp
        .create_group(Parameters(CreateGroupArgs {
            prompt: "Commentable".to_string(),
            title: None,
            parent_group_id: None,
            tag_ids: None,
            canvas_id: None,
        }))
        .await
        .unwrap();
    let group_id = result_json(&g)["group"]["id"].as_str().unwrap().to_string();

    let res = mcp
        .add_comment(Parameters(shape_server::mcp::AddCommentArgs {
            target: json!({ "kind": "group", "id": group_id }),
            body: "looks good".to_string(),
            author: None,
            canvas_id: None,
        }))
        .await
        .expect("add_comment");
    let res = result_json(&res);
    assert_eq!(res["comment"]["body"], "looks good");
    assert_eq!(res["scene"]["comments"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn trace_records_tool_activity_for_the_client() {
    let (mcp, clients) = mcp_instance();
    // Register the client so its ring exists (the transport does this on init).
    clients.register("test-client", "tester", "0.1", "http");

    mcp.query_scene(Parameters(QuerySceneArgs::default()))
        .await
        .unwrap();
    mcp.create_group(Parameters(CreateGroupArgs {
        prompt: "traced".to_string(),
        title: None,
        parent_group_id: None,
        tag_ids: None,
        canvas_id: None,
    }))
    .await
    .unwrap();

    let trace = mcp
        .get_client_trace(Parameters(GetClientTraceArgs {
            client_id: "test-client".to_string(),
            limit: None,
        }))
        .await
        .unwrap();
    let trace = result_json(&trace);
    let entries = trace["trace"].as_array().unwrap();
    assert!(entries.len() >= 2, "read + write traced, got {entries:?}");
    // Newest first: the create_group write is on top.
    assert_eq!(entries[0]["verb"], "create_group");
}

#[tokio::test]
async fn dock_clients_and_trace_endpoints_return_json() {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let clients = ClientRegistry::new();
    clients.register("c1", "claude", "1.0", "http");
    clients.push_trace(
        "c1",
        shape_server::mcp_clients::TraceKind::Write,
        "create-group",
        "made g".to_string(),
        None,
    );

    let app = build_router_with_mcp(&test_config(), canvases, clients);

    // /api/mcp/clients
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/mcp/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let list = body["clients"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["clientId"], "c1");
    assert_eq!(list[0]["label"], "claude");
    assert_eq!(list[0]["color"], shape_server::mcp_clients::color_from_client_id("c1"));

    // /api/mcp/trace?clientId=c1
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/mcp/trace?clientId=c1&limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["clientId"], "c1");
    assert_eq!(body["total"], 1);
    assert_eq!(body["trace"][0]["verb"], "create-group");
}
